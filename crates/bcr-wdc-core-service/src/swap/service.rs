// ----- standard library imports
use std::collections::HashSet;
// ----- extra library imports
use bcr_common::{
    cashu,
    core::{signature, swap},
    wire::{attestation as wire_attestation, swap as wire_swap},
};
use bcr_wdc_utils::signatures as signatures_utils;
use bitcoin::secp256k1::PublicKey;
use futures::future::JoinAll;
use itertools::izip;
use secp256k1::schnorr;
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence::{CommitmentRepository, ProofRepository, ReservedYsRepository, StoredCommitment},
    swap::{ClowderClient, KeysService, TreasuryService},
    TStamp,
};

// ----- end imports

pub struct Service {
    pub proofs: Box<dyn ProofRepository>,
    pub commitments: Box<dyn CommitmentRepository>,
    pub reserved: Box<dyn ReservedYsRepository>,
    pub clowder: Box<dyn ClowderClient>,
    pub treasury: Box<dyn TreasuryService>,
    pub max_expiry: chrono::Duration,
    pub alpha_id: PublicKey,
}

impl Service {
    pub async fn check_state(
        &self,
        ys: &[cashu::PublicKey],
        now: TStamp,
    ) -> Result<Vec<cashu::ProofState>> {
        self.commitments.clean_expired(now).await?;
        self.reserved.clean_expired(now).await?;
        let joined_spent = ys
            .iter()
            .map(|y| self.proofs.contains(*y))
            .collect::<JoinAll<_>>();
        let states: Vec<_> = joined_spent.await.into_iter().collect::<Result<_>>()?;
        let reserveds = self.reserved.contains(ys).await?;
        let mut proof_states = Vec::with_capacity(states.len());
        for (state, reserved, y) in izip!(states.into_iter(), reserveds.into_iter(), ys.iter()) {
            if let Some(state) = state {
                proof_states.push(state);
            } else if reserved {
                proof_states.push(cashu::ProofState {
                    y: *y,
                    state: cashu::State::Reserved,
                    witness: None,
                });
            } else {
                let committed = self
                    .commitments
                    .contains_inputs(std::slice::from_ref(y))
                    .await?;
                if committed {
                    proof_states.push(cashu::ProofState {
                        y: *y,
                        state: cashu::State::Reserved,
                        witness: None,
                    });
                } else {
                    let state = cashu::ProofState {
                        y: *y,
                        state: cashu::State::Unspent,
                        witness: None,
                    };
                    proof_states.push(state);
                }
            }
        }
        Ok(proof_states)
    }

    pub async fn signed_commit_to_swap(
        &self,
        sign_service: &dyn KeysService,
        payload: String,
        signature: schnorr::Signature,
        now: TStamp,
    ) -> Result<(String, schnorr::Signature)> {
        let content: wire_swap::SwapCommitmentRequest = signature::deserialize_borsh_msg(&payload)?;
        signature::schnorr_verify_b64(
            &payload,
            &signature,
            &content.wallet_key.x_only_public_key().0,
        )?;
        let _owner = self.clowder.verify_pk(&content.wallet_key).await?;
        self._commit_to_swap(sign_service, content, now, true).await
    }

    pub async fn commit_to_swap(
        &self,
        sign_service: &dyn KeysService,
        request: wire_swap::SwapCommitmentRequest,
        now: TStamp,
    ) -> Result<(String, schnorr::Signature)> {
        self._commit_to_swap(sign_service, request, now, false)
            .await
    }

    async fn _commit_to_swap(
        &self,
        sign_service: &dyn KeysService,
        request: wire_swap::SwapCommitmentRequest,
        now: TStamp,
        signed: bool,
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
            .inputs
            .iter()
            .map(|fp| signature::ProofFingerprint::from(fp.clone()))
            .collect::<Vec<_>>();
        signatures_utils::basic_fingerprints_checks(&core_fps)?;
        signatures_utils::basic_blinds_checks(&request.outputs)?;
        // authenticate the Beta issuance attestation bound to this commitment
        self.clowder
            .authenticate_attestation(&self.alpha_id, &request.inputs)
            .await?;
        // verify inputs/outputs amounts
        let kinfos = sign_service.list_kinfos().await?;
        swap::mint::verify_commit(&core_fps, &request.outputs, &kinfos)?;
        // check inputs are unspent
        let ys: Vec<cashu::PublicKey> = request.inputs.inputs.iter().map(|fp| fp.y).collect();
        let states = self.check_state(&ys, now).await?;
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
        let fp_digest = request.inputs.attestation.fp_digest;
        let (content, commitment) = self.clowder.commit_to_swap(request).await?;
        // store commitment
        let store_res = self
            .commitments
            .store(
                ys,
                bs,
                expiry,
                wallet_key.into(),
                commitment,
                fp_digest,
                signed,
            )
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
        commitment: schnorr::Signature,
        now: TStamp,
    ) -> Result<Vec<cashu::BlindSignature>> {
        // cheap verifications
        signatures_utils::basic_proofs_checks(&inputs)?;
        signatures_utils::basic_blinds_checks(&outputs)?;
        // cross check with commitment
        let StoredCommitment {
            outputs: committed_outputs,
            expiration,
            fp_digest: committed_fp_digest,
            signed,
            ..
        } = self.commitments.load(&commitment).await?;
        // check expiration
        if expiration < now {
            return Err(Error::InvalidInput(String::from("commitment has expired")));
        }
        // swap inputs must project to the fingerprints attested at commitment time
        let input_fps = wire_attestation::project_to_fingerprints(&inputs)?;
        if wire_attestation::fp_digest(&input_fps) != committed_fp_digest {
            return Err(Error::Attestation(
                wire_attestation::AttestationError::DigestMismatch,
            ));
        }
        // committed and swap outputs must be equal
        let output_bs: Vec<cashu::PublicKey> =
            outputs.iter().map(|b| b.blinded_secret).collect::<Vec<_>>();
        let checked = cross_check_commits_swaps(&committed_outputs, &output_bs);
        if !checked {
            return Err(Error::InvalidInput(format!(
                "output/committed_outputs mismatch {:?}/{:?}",
                output_bs, committed_outputs
            )));
        }
        let (kinfos, _) = tokio::try_join!(
            sign_service.list_kinfos(),
            sign_service.verify_proofs(&inputs),
        )?;
        let fee_policy = match signed {
            true => swap::mint::FeePolicy::Ignore,
            false => swap::mint::FeePolicy::Apply,
        };
        swap::mint::verify_swap(&inputs, &outputs, &kinfos, fee_policy)?;
        // generate signatures
        let signatures = sign_service.sign_blinds(&outputs).await?;
        let fees_premints = generate_fees_premints(sign_service, &inputs, &outputs).await?;
        let (fees_signatures, fees_proofs) = sign_fees(sign_service, fees_premints).await?;
        // signal swap to clowder
        self.clowder
            .signal_swap_event(
                inputs.clone(),
                outputs,
                fees_signatures,
                commitment,
                signatures.clone(),
            )
            .await?;
        // update state
        self.commitments.delete(commitment).await?;
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

    pub async fn reserve(&self, ys: Vec<cashu::PublicKey>, deadline: TStamp) -> Result<()> {
        self.reserved.store(ys, deadline).await
    }
}

async fn generate_fees_premints(
    signer: &dyn KeysService,
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
        let keyset = signer.get_keyset(&kid).await?;
        let premint = cashu::PreMintSecrets::random(
            kid,
            inputs_amount - outputs_amount,
            &cashu::amount::SplitTarget::None,
            &bcr_wdc_utils::keys::to_fee_and_amounts(&keyset),
        )?;
        premints.push(premint);
    }
    Ok(premints)
}

async fn sign_fees(
    signer: &dyn KeysService,
    premints: Vec<cashu::PreMintSecrets>,
) -> Result<(Vec<cashu::BlindSignature>, Vec<cashu::Proof>)> {
    if premints.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
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

fn cross_check_commits_swaps<T: PartialEq>(committed: &[T], swap: &[T]) -> bool {
    if committed.len() != swap.len() {
        return false;
    }
    for c in committed.iter() {
        let present = swap.iter().any(|s| s == c);
        if !present {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        persistence::{MockCommitmentRepository, MockProofRepository, MockReservedYsRepository},
        swap::MockTreasuryService,
        swap::{test_utils::DummyTreasuryClient, MockClowderClient, MockKeysService},
        test_utils::dummy_attestation,
    };
    use bcr_common::{core, core_tests, wire::attestation::AttestationError};
    use bcr_wdc_utils::signatures::test_utils as signatures_test;
    use mockall::predicate::eq;

    #[tokio::test]
    async fn commit_rejects_when_attestation_invalid() {
        let mut clowder = MockClowderClient::new();
        let commitments = MockCommitmentRepository::new();
        let proofs_repo = MockProofRepository::new();
        let sign_service = MockKeysService::new();
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts = [cashu::Amount::from(8u64)];
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        let blinds: Vec<_> = signatures_test::generate_blinds(keyset.id, &amounts)
            .into_iter()
            .map(|b| b.0)
            .collect();
        let fps: Vec<_> = proofs
            .iter()
            .cloned()
            .map(|p| bcr_common::wire::keys::ProofFingerprint::try_from(p).unwrap())
            .collect();
        // attestation is authenticated at commitment time, before any other work
        clowder
            .expect_authenticate_attestation()
            .times(1)
            .returning(|_, _| Err(Error::Attestation(AttestationError::DigestMismatch)));
        let alpha_id = core::generate_random_keypair().public_key();
        let service = Service {
            proofs: Box::new(proofs_repo),
            commitments: Box::new(commitments),
            reserved: Box::new(MockReservedYsRepository::new()),
            clowder: Box::new(clowder),
            treasury: Box::new(DummyTreasuryClient),
            max_expiry: chrono::Duration::seconds(3600),
            alpha_id,
        };
        let request = wire_swap::SwapCommitmentRequest {
            inputs: bcr_common::wire::attestation::AttestedFingerprints {
                inputs: fps,
                attestation: dummy_attestation(),
            },
            outputs: blinds,
            expiry: (chrono::Utc::now() + chrono::Duration::seconds(60)).timestamp() as u64,
            wallet_key: core::generate_random_keypair().public_key(),
        };
        let err = service
            .commit_to_swap(&sign_service, request, chrono::Utc::now())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::Attestation(AttestationError::DigestMismatch)
        ));
    }

    #[tokio::test]
    async fn signed_commit_to_swap_unknown_signer() {
        let mut clowder = MockClowderClient::new();
        let commitments = MockCommitmentRepository::new();
        let proofs_repo = MockProofRepository::new();
        let sign_service = MockKeysService::new();
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts = [cashu::Amount::from(8u64)];
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        let blinds: Vec<_> = signatures_test::generate_blinds(keyset.id, &amounts)
            .into_iter()
            .map(|b| b.0)
            .collect();
        let fps: Vec<_> = proofs
            .iter()
            .cloned()
            .map(|p| bcr_common::wire::keys::ProofFingerprint::try_from(p).unwrap())
            .collect();
        // attestation is authenticated at commitment time, before any other work
        clowder
            .expect_verify_pk()
            .times(1)
            .returning(|_| Err(Error::InvalidInput(String::new())));
        let alpha_kp = core::generate_random_keypair();
        let service = Service {
            proofs: Box::new(proofs_repo),
            commitments: Box::new(commitments),
            reserved: Box::new(MockReservedYsRepository::new()),
            clowder: Box::new(clowder),
            treasury: Box::new(DummyTreasuryClient),
            max_expiry: chrono::Duration::seconds(3600),
            alpha_id: alpha_kp.public_key(),
        };
        let request = wire_swap::SwapCommitmentRequest {
            inputs: bcr_common::wire::attestation::AttestedFingerprints {
                inputs: fps,
                attestation: dummy_attestation(),
            },
            outputs: blinds,
            expiry: (chrono::Utc::now() + chrono::Duration::seconds(60)).timestamp() as u64,
            wallet_key: alpha_kp.public_key(),
        };
        let (content, signature) =
            signature::serialize_n_schnorr_sign_borsh_msg(&request, &alpha_kp).unwrap();
        let err = service
            .signed_commit_to_swap(&sign_service, content, signature, chrono::Utc::now())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[tokio::test]
    async fn swap_1sat_no_output() {
        let mut clowder = MockClowderClient::new();
        let mut commitments = MockCommitmentRepository::new();
        let mut proofs_repo = MockProofRepository::new();
        let mut sign_service = MockKeysService::new();
        let mut mocktreasu = MockTreasuryService::new();
        let (mut kinfo, keyset) = core_tests::generate_random_ecash_keyset();
        kinfo.input_fee_ppk = 1;
        let amounts = [cashu::Amount::from(1u64)];
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        let commitment = bitcoin::secp256k1::schnorr::Signature::from_slice(&[0; 64]).unwrap();
        let proof_ys: Vec<cashu::PublicKey> = proofs.iter().map(|p| p.y().unwrap()).collect();
        let fp_digest = wire_attestation::fp_digest(
            &wire_attestation::project_to_fingerprints(&proofs).unwrap(),
        );
        let expiry = chrono::Utc::now() + chrono::Duration::seconds(60);
        commitments.expect_load().times(1).returning(move |_| {
            Ok(StoredCommitment {
                inputs: proof_ys.clone(),
                outputs: vec![],
                expiration: expiry,
                fp_digest,
                signed: false,
            })
        });
        let cloned = cashu::KeySetInfo::from(kinfo);
        sign_service.expect_list_kinfos().returning(move || {
            Ok(std::collections::HashMap::from([(
                keyset.id,
                cloned.clone(),
            )]))
        });
        sign_service
            .expect_verify_proofs()
            .times(1)
            .returning(|_| Ok(()));
        sign_service
            .expect_sign_blinds()
            .times(1)
            .with(eq(vec![]))
            .returning(move |_| Ok(vec![]));
        let cloned_set = bcr_common::core::keys::to_keyset(&keyset, Some(true));
        sign_service
            .expect_get_keyset()
            .times(2)
            .returning(move |_| Ok(cloned_set.clone()));
        let cloned_set = keyset.clone();
        sign_service
            .expect_sign_blinds()
            .times(1)
            .returning(move |bs| {
                let mut signs = Vec::with_capacity(bs.len());
                for b in bs {
                    let sign = bcr_common::core::signature::sign_ecash(&cloned_set, b).unwrap();
                    signs.push(sign);
                }
                Ok(signs)
            });
        clowder
            .expect_signal_swap_event()
            .times(1)
            .returning(|_, _, _, _, _| Ok(()));
        commitments.expect_delete().times(1).returning(|_| Ok(()));
        proofs_repo.expect_insert().times(1).returning(|_| Ok(()));
        mocktreasu
            .expect_store_proofs()
            .times(1)
            .returning(|_| Ok(()));
        let alpha_id = core::generate_random_keypair().public_key();
        let service = Service {
            proofs: Box::new(proofs_repo),
            commitments: Box::new(commitments),
            reserved: Box::new(MockReservedYsRepository::new()),
            clowder: Box::new(clowder),
            treasury: Box::new(mocktreasu),
            max_expiry: chrono::Duration::seconds(3600),
            alpha_id,
        };
        let signatures = service
            .swap(
                &sign_service,
                proofs,
                vec![],
                commitment,
                chrono::Utc::now(),
            )
            .await
            .unwrap();
        assert!(signatures.is_empty());
    }

    #[tokio::test]
    async fn swap_rejects_on_digest_mismatch() {
        let clowder = MockClowderClient::new();
        let mut commitments = MockCommitmentRepository::new();
        let proofs_repo = MockProofRepository::new();
        let sign_service = MockKeysService::new();
        let mocktreasu = MockTreasuryService::new();
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts = [cashu::Amount::from(8u64)];
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        let commitment = bitcoin::secp256k1::schnorr::Signature::from_slice(&[0; 64]).unwrap();
        let proof_ys: Vec<cashu::PublicKey> = proofs.iter().map(|p| p.y().unwrap()).collect();
        let expiry = chrono::Utc::now() + chrono::Duration::seconds(60);
        // commitment carries a digest over different fingerprints
        commitments.expect_load().times(1).returning(move |_| {
            Ok(StoredCommitment {
                inputs: proof_ys.clone(),
                outputs: vec![],
                expiration: expiry,
                fp_digest: [1u8; 32],
                signed: false,
            })
        });
        let alpha_id = core::generate_random_keypair().public_key();
        let service = Service {
            proofs: Box::new(proofs_repo),
            commitments: Box::new(commitments),
            reserved: Box::new(MockReservedYsRepository::new()),
            clowder: Box::new(clowder),
            treasury: Box::new(mocktreasu),
            max_expiry: chrono::Duration::seconds(3600),
            alpha_id,
        };
        let err = service
            .swap(
                &sign_service,
                proofs,
                vec![],
                commitment,
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
