// ----- standard library imports
use std::collections::HashSet;
// ----- extra library imports
use bcr_common::{
    cashu,
    core::{signature, swap},
    wire::{attestation::IssuanceAttestation, swap as wire_swap},
};
use bcr_wdc_utils::signatures as signatures_utils;
use bitcoin::secp256k1::PublicKey;
use futures::future::JoinAll;
use secp256k1::schnorr;
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence::{CommitmentRepository, ProofRepository},
    swap::{ClowderClient, KeysService, TreasuryService},
    TStamp,
};

// ----- end imports

pub struct Service {
    pub proofs: Box<dyn ProofRepository>,
    pub commitments: Box<dyn CommitmentRepository>,
    pub clowder: Box<dyn ClowderClient>,
    pub treasury: Box<dyn TreasuryService>,
    pub max_expiry: chrono::Duration,
    pub alpha_id: PublicKey,
}

impl Service {
    pub async fn check_spendable(&self, ys: &[cashu::PublicKey]) -> Result<Vec<cashu::ProofState>> {
        let joined = ys
            .iter()
            .map(|y| self.proofs.contains(*y))
            .collect::<JoinAll<_>>();
        let responses: Vec<_> = joined.await.into_iter().collect::<Result<_>>()?;
        let mut proof_states = Vec::with_capacity(responses.len());
        for (response, y) in responses.into_iter().zip(ys.iter()) {
            let proof_state = response.unwrap_or(cashu::ProofState {
                y: *y,
                state: cashu::State::Unspent,
                witness: None,
            });
            proof_states.push(proof_state);
        }
        Ok(proof_states)
    }

    pub async fn commit_to_swap(
        &self,
        sign_service: &dyn KeysService,
        request: wire_swap::SwapCommitmentRequest,
        now: TStamp,
    ) -> Result<(String, schnorr::Signature)> {
        // check expiry
        let expiry = chrono::DateTime::from_timestamp(request.expiry as i64, 0)
            .ok_or_else(|| Error::InvalidInput("invalid expiry timestamp".into()))?;
        if expiry < now {
            return Err(Error::InvalidInput("commitment already expired".into()));
        }
        let max_allowed = now + self.max_expiry;
        let expiry = expiry.min(max_allowed);
        // basic checks
        let core_fps = request
            .inputs
            .iter()
            .map(|fp| signature::ProofFingerprint::from(fp.clone()))
            .collect::<Vec<_>>();
        signatures_utils::basic_fingerprints_checks(&core_fps)?;
        signatures_utils::basic_blinds_checks(&request.outputs)?;
        let kinfos = sign_service.list_kinfos().await?;
        swap::mint::verify_commit(&core_fps, &request.outputs, &kinfos)?;
        // check inputs are unspent
        let ys: Vec<cashu::PublicKey> = request.inputs.iter().map(|fp| fp.y).collect();
        let states = self.check_spendable(&ys).await?;
        let all_unspent = states
            .iter()
            .all(|s| matches!(s.state, cashu::State::Unspent));
        if !all_unspent {
            return Err(Error::InvalidInput(
                "One or more proofs are not unspent".to_string(),
            ));
        }
        // check inputs signatures
        sign_service.verify_fingerprints(&core_fps).await?;
        // check inputs not already committed
        self.commitments.clean_expired(now).await?;
        let contained = self.commitments.contains_inputs(&ys).await?;
        if contained {
            return Err(Error::InvalidInput(String::from("proofs committed")));
        }
        // check outputs not already committed
        let bs: Vec<cashu::PublicKey> = request.outputs.iter().map(|b| b.blinded_secret).collect();
        let contained = self.commitments.contains_outputs(&bs).await?;
        if contained {
            return Err(Error::InvalidInput(String::from(
                "blinded messages committed",
            )));
        }
        // broadcast request to clowder, get back mint commitment
        let wallet_key = request.wallet_key;
        let (content, commitment) = self.clowder.commit_to_swap(request).await?;
        // store commitment
        let store_res = self
            .commitments
            .store(ys, bs, expiry, wallet_key.into(), commitment)
            .await;
        match store_res {
            Ok(_) => Ok((content, commitment)),
            Err(e) => {
                tracing::error!("failed to store commitment: {e}");
                Err(e)
            }
        }
    }

    pub async fn swap(
        &self,
        sign_service: &dyn KeysService,
        inputs: Vec<cashu::Proof>,
        outputs: Vec<cashu::BlindedMessage>,
        signature: schnorr::Signature,
        attestation: IssuanceAttestation,
        now: TStamp,
    ) -> Result<Vec<cashu::BlindSignature>> {
        // cheap verifications
        signatures_utils::basic_proofs_checks(&inputs)?;
        signatures_utils::basic_blinds_checks(&outputs)?;
        // cross check with commitment
        let (committed_inputs, committed_outputs, expiration) =
            self.commitments.load(&signature).await?;
        // check expiration
        if expiration < now {
            return Err(Error::InvalidInput(String::from("commitment has expired")));
        }
        // committed and swap inputs must be equal
        if committed_inputs.len() != inputs.len() {
            return Err(Error::InvalidInput(String::from(
                "inputs/committed_inputs mismatch",
            )));
        }
        for input in inputs.iter() {
            let y = input.y()?;
            committed_inputs
                .iter()
                .find(|committed| **committed == y)
                .ok_or_else(|| {
                    Error::InvalidInput(format!(
                        "input/committed_input mismatch {y}/{:?}",
                        committed_inputs,
                    ))
                })?;
        }
        // committed and swap outputs must be equal
        if committed_outputs.len() != outputs.len() {
            return Err(Error::InvalidInput(String::from(
                "outputs/committed_outputs mismatch",
            )));
        }
        for output in outputs.iter() {
            let b = output.blinded_secret;
            committed_outputs
                .iter()
                .find(|commited| **commited == b)
                .ok_or_else(|| {
                    Error::InvalidInput(format!(
                        "output/committed_output mismatch {b}/{:?}",
                        committed_outputs,
                    ))
                })?;
        }
        // commitment pins inputs by `y`; attestation pins them by `fp_digest` over the same proofs.
        let (verify_res, kinfos_res) = tokio::join!(
            self.clowder
                .verify_attestation(&self.alpha_id, &inputs, &attestation),
            sign_service.list_kinfos(),
        );
        verify_res?;
        let kinfos = kinfos_res?;
        swap::mint::verify_swap(&inputs, &outputs, &kinfos)?;
        sign_service.verify_proofs(&inputs).await?;
        // check inputs are unspent
        let ys: Vec<cashu::PublicKey> = inputs
            .iter()
            .map(|fp| fp.y())
            .collect::<std::result::Result<_, _>>()?;
        let states = self.check_spendable(&ys).await?;
        let all_unspent = states
            .iter()
            .all(|s| matches!(s.state, cashu::State::Unspent));
        if !all_unspent {
            return Err(Error::InvalidInput(
                "One or more proofs are not unspent".to_string(),
            ));
        }
        // generate signatures
        let signatures = sign_service.sign_blinds(&outputs).await?;
        let fees_premints = generate_fees_premints(&inputs, &outputs)?;
        let (fees_signatures, fees_proofs) = sign_fees(sign_service, fees_premints).await?;
        // signal swap to clowder
        self.clowder
            .signal_swap_event(
                inputs.clone(),
                outputs,
                fees_signatures,
                signature,
                attestation,
                signatures.clone(),
            )
            .await?;
        // update state
        self.commitments.delete(signature).await?;
        self.proofs.insert(inputs).await?;
        self.treasury.store_proofs(fees_proofs).await?;
        Ok(signatures)
    }

    pub async fn burn(
        &self,
        sign_service: &dyn KeysService,
        proofs: Vec<cashu::Proof>,
    ) -> Result<Vec<cashu::PublicKey>> {
        // cheap verifications
        signatures_utils::basic_proofs_checks(&proofs)?;
        // verify proofs signatures
        sign_service.verify_proofs(&proofs).await?;
        let mut ys = Vec::with_capacity(proofs.len());
        for proof in &proofs {
            let y = cashu::dhke::hash_to_curve(proof.secret.as_bytes())?;
            ys.push(y);
        }
        self.proofs.insert(proofs).await?;
        Ok(ys)
    }

    pub async fn recover(&self, proofs: &[cashu::Proof]) -> Result<()> {
        let ys = proofs
            .iter()
            .map(|proof| cashu::dhke::hash_to_curve(proof.secret.as_bytes()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.proofs.remove(&ys).await?;
        Ok(())
    }
}

fn generate_fees_premints(
    inputs: &[cashu::Proof],
    outputs: &[cashu::BlindedMessage],
) -> Result<Vec<cashu::PreMintSecrets>> {
    let unique_kids: HashSet<_> = inputs.iter().map(|proof| proof.keyset_id).collect();
    let mut premints = Vec::with_capacity(unique_kids.len());
    for kid in unique_kids {
        let inputs_amount = inputs
            .iter()
            .filter(|proof| proof.keyset_id == kid)
            .fold(cashu::Amount::ZERO, |acc, proof| acc + proof.amount);
        let outputs_amount = outputs
            .iter()
            .filter(|b| b.keyset_id == kid)
            .fold(cashu::Amount::ZERO, |acc, b| acc + b.amount);
        if inputs_amount <= outputs_amount {
            continue;
        }
        let premint = cashu::PreMintSecrets::random(
            kid,
            inputs_amount - outputs_amount,
            &cashu::amount::SplitTarget::None,
        )?;
        premints.push(premint);
    }
    Ok(premints)
}

async fn sign_fees(
    signer: &dyn KeysService,
    premints: Vec<cashu::PreMintSecrets>,
) -> Result<(Vec<cashu::BlindSignature>, Vec<cashu::Proof>)> {
    let total_len = premints.iter().map(|p| p.len()).sum();
    let mut signatures = Vec::with_capacity(total_len);
    let mut proofs = Vec::with_capacity(total_len);
    for premint in premints {
        let keyset = signer.get_keyset(&premint.keyset_id).await?;
        let signs = signer.sign_blinds(&premint.blinded_messages()).await?;
        let (rs, secrets) = premint
            .secrets
            .into_iter()
            .map(|premint| (premint.r, premint.secret))
            .unzip();
        let prfs = cashu::dhke::construct_proofs(signs.clone(), rs, secrets, &keyset.keys)?;
        signatures.extend(signs);
        proofs.extend(prfs);
    }
    Ok((signatures, proofs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::dummy_attestation;
    use crate::{
        persistence::{MockCommitmentRepository, MockProofRepository},
        swap::{test_utils::DummyTreasuryClient, MockClowderClient, MockKeysService},
    };
    use bcr_common::{core_tests, wire::attestation::AttestationError};
    use bcr_wdc_utils::signatures::test_utils as signatures_test;

    #[tokio::test]
    async fn swap_rejects_when_attestation_invalid() {
        let mut clowder = MockClowderClient::new();
        let mut commitments = MockCommitmentRepository::new();
        let proofs_repo = MockProofRepository::new();
        let mut sign_service = MockKeysService::new();

        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts = [cashu::Amount::from(8u64)];
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        let blinds: Vec<_> = signatures_test::generate_blinds(keyset.id, &amounts)
            .into_iter()
            .map(|b| b.0)
            .collect();
        let commitment = bitcoin::secp256k1::schnorr::Signature::from_slice(&[0; 64]).unwrap();

        let proof_ys: Vec<cashu::PublicKey> = proofs.iter().map(|p| p.y().unwrap()).collect();
        let blind_bs: Vec<cashu::PublicKey> = blinds.iter().map(|b| b.blinded_secret).collect();
        let expiry = chrono::Utc::now() + chrono::Duration::seconds(60);
        commitments
            .expect_load()
            .times(1)
            .returning(move |_| Ok((proof_ys.clone(), blind_bs.clone(), expiry)));

        clowder
            .expect_verify_attestation()
            .times(1)
            .returning(|_, _, _| Err(Error::Attestation(AttestationError::DigestMismatch)));

        sign_service
            .expect_list_kinfos()
            .returning(|| Ok(std::collections::HashMap::new()));
        sign_service.expect_sign_blinds().times(0);

        let alpha_id = core_tests::generate_random_keypair().public_key();
        let service = Service {
            proofs: Box::new(proofs_repo),
            commitments: Box::new(commitments),
            clowder: Box::new(clowder),
            treasury: Box::new(DummyTreasuryClient),
            max_expiry: chrono::Duration::seconds(3600),
            alpha_id,
        };

        let err = service
            .swap(
                &sign_service,
                proofs,
                blinds,
                commitment,
                dummy_attestation(),
                chrono::Utc::now(),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::Attestation(AttestationError::DigestMismatch)
        ));
    }
}
