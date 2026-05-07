// ----- standard library imports
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
    swap::{ClowderClient, KeysService},
    TStamp,
};

// ----- end imports

pub struct Service {
    pub proofs: Box<dyn ProofRepository>,
    pub commitments: Box<dyn CommitmentRepository>,
    pub clowder: Box<dyn ClowderClient>,
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
        // verify Beta-issued attestation before any signing
        self.clowder
            .verify_attestation(&self.alpha_id, &inputs, &attestation)
            .await?;
        // verify inputs
        let kinfos = sign_service.list_kinfos().await?;
        swap::mint::verify_swap(&inputs, &outputs, &kinfos)?;
        sign_service.verify_proofs(&inputs).await?;
        // generate signatures
        let signatures = sign_service.sign_blinds(&outputs).await?;
        self.proofs.insert(&inputs).await?;
        self.clowder
            .post_swap(inputs, outputs, signature, attestation, signatures.clone())
            .await?;
        let res = self.commitments.delete(signature).await;
        if let Err(e) = res {
            tracing::error!("failed to delete commitment: {e}");
        }
        Ok(signatures)
    }

    pub async fn burn(
        &self,
        sign_service: &dyn KeysService,
        proofs: &[cashu::Proof],
    ) -> Result<Vec<cashu::PublicKey>> {
        // cheap verifications
        signatures_utils::basic_proofs_checks(proofs)?;
        // verify proofs signatures
        sign_service.verify_proofs(proofs).await?;
        let mut ys = Vec::with_capacity(proofs.len());
        for proof in proofs {
            let y = cashu::dhke::hash_to_curve(proof.secret.as_bytes())?;
            ys.push(y);
        }
        self.proofs.insert(proofs).await?;
        Ok(ys)
    }

    pub async fn recover(&self, proofs: &[cashu::Proof]) -> Result<()> {
        self.proofs.remove(proofs).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        persistence::{MockCommitmentRepository, MockProofRepository},
        swap::{MockClowderClient, MockKeysService},
    };
    use bcr_common::{
        core_tests,
        wire::attestation::{AttestationError, IssuanceAttestation},
    };
    use bcr_wdc_utils::signatures::test_utils as signatures_test;

    fn dummy_attestation() -> IssuanceAttestation {
        let kp = core_tests::generate_random_keypair();
        let signature = bitcoin::secp256k1::schnorr::Signature::from_slice(&[0; 64]).unwrap();
        IssuanceAttestation {
            beta_id: kp.public_key(),
            fp_digest: [0u8; 32],
            coords_mac: [0u8; 32],
            signature,
        }
    }

    /// If attestation verification fails, swap must short-circuit before
    /// signing or burning.
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

        let proof_ys: Vec<cashu::PublicKey> =
            proofs.iter().map(|p| p.y().unwrap()).collect();
        let blind_bs: Vec<cashu::PublicKey> =
            blinds.iter().map(|b| b.blinded_secret).collect();
        let expiry = chrono::Utc::now() + chrono::Duration::seconds(60);
        commitments
            .expect_load()
            .times(1)
            .returning(move |_| Ok((proof_ys.clone(), blind_bs.clone(), expiry)));

        clowder
            .expect_verify_attestation()
            .times(1)
            .returning(|_, _, _| Err(Error::Attestation(AttestationError::DigestMismatch)));

        // Signing/insert/post must NOT be called when attestation fails.
        sign_service.expect_sign_blinds().times(0);

        let alpha_id = core_tests::generate_random_keypair().public_key();
        let service = Service {
            proofs: Box::new(proofs_repo),
            commitments: Box::new(commitments),
            clowder: Box::new(clowder),
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
