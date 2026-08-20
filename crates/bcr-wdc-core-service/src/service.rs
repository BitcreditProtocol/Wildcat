// ----- standard library imports
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
// ----- extra library imports
use bcr_common::{
    cashu,
    cdk_common::mint::MintKeySetInfo,
    client::admin::core::{BRError, RNFError},
    core::{
        signature::{
            self, sign_ecash, verify_ecash_fingerprint, verify_ecash_proof, ProofFingerprint,
        },
        swap,
    },
    wire::{attestation as wire_attestation, swap as wire_swap},
};
use bcr_wdc_utils::signatures as signatures_utils;
use bitcoin::secp256k1::PublicKey;
use futures::future::JoinAll;
use itertools::izip;
use secp256k1::schnorr;
// ----- local imports
use crate::{
    clients::ClowderClient,
    error::{Error, Result},
    keys::factory::Factory,
    persistence::{Repository, SignatureOwner, StoredCommitment, StoredSignature},
    swap::TreasuryService,
    TStamp,
};

// ----- end imports

#[derive(Default)]
pub struct ListFilters {
    pub unit: Option<cashu::CurrencyUnit>,
    pub min_expiration: Option<chrono::NaiveDate>,
    pub max_expiration: Option<chrono::NaiveDate>,
}

pub struct Service {
    pub repository: Arc<dyn Repository>,
    pub clowder: Box<dyn ClowderClient>,
    pub treasury: Box<dyn TreasuryService>,
    pub keygen: Factory,
    pub min_keyset_fees_ppk: AtomicU64,
    pub max_expiry: chrono::Duration,
    pub alpha_id: PublicKey,
    pub settle_window_deadline: TStamp,
}

impl Service {
    pub fn set_minimum_fees_ppk(&self, fees_ppk: u64) -> Result<()> {
        self.min_keyset_fees_ppk.store(fees_ppk, Ordering::Relaxed);
        Ok(())
    }

    pub async fn create(
        &self,
        unit: cashu::CurrencyUnit,
        now: TStamp,
        expiration: Option<TStamp>,
        fees_ppk: u64,
    ) -> Result<MintKeySetInfo> {
        let fees_ppk = std::cmp::max(fees_ppk, self.min_keyset_fees_ppk.load(Ordering::Relaxed));
        let entry = self.keygen.generate(unit, now, expiration, fees_ppk);
        let kinfo = entry.0.clone();
        let keyset = bcr_wdc_utils::keys::to_keyset(&entry.1, Some(entry.0.active));
        self.clowder.new_keyset(keyset).await?;
        self.repository.keys_store(entry).await?;
        Ok(kinfo)
    }

    pub async fn info(&self, kid: cashu::Id) -> Result<MintKeySetInfo> {
        self.repository
            .keys_info(kid)
            .await?
            .ok_or(Error::ResourceNotFound(RNFError::KeysetId(kid)))
    }

    pub async fn keys(&self, kid: cashu::Id) -> Result<cashu::MintKeySet> {
        self.repository
            .keys_load(kid)
            .await?
            .ok_or(Error::ResourceNotFound(RNFError::KeysetId(kid)))
    }

    pub async fn verify_proofs(&self, proofs: &[cashu::Proof]) -> Result<()> {
        let by_kid = bcr_common::core::signature::proofs_to_map(proofs.iter().cloned());
        for (kid, proofs) in by_kid {
            let keyset = self.keys(kid).await?;
            for proof in &proofs {
                verify_ecash_proof(&keyset, proof)?;
            }
        }
        Ok(())
    }

    pub async fn verify_fingerprints(&self, fps: &[ProofFingerprint]) -> Result<()> {
        let by_kid: HashMap<cashu::Id, Vec<&ProofFingerprint>> =
            fps.iter().fold(HashMap::new(), |mut kmap, fp| {
                kmap.entry(fp.keyset_id).or_default().push(fp);
                kmap
            });
        for (kid, fps) in by_kid {
            let keyset = self.keys(kid).await?;
            for fp in fps {
                verify_ecash_fingerprint(&keyset, fp)?;
            }
        }
        Ok(())
    }

    pub async fn list_info(&self, filters: ListFilters) -> Result<Vec<MintKeySetInfo>> {
        let min_tstamp = filters
            .min_expiration
            .map(|date| date.and_time(chrono::NaiveTime::MIN).and_utc().timestamp() as u64);
        let max_tstamp = filters
            .max_expiration
            .map(|date| date.and_time(chrono::NaiveTime::MIN).and_utc().timestamp() as u64);
        self.repository
            .keys_list_info(filters.unit, min_tstamp, max_tstamp)
            .await
    }

    pub async fn search_signature(
        &self,
        blind: &cashu::BlindedMessage,
    ) -> Result<Option<cashu::BlindSignature>> {
        self.repository.signature_load(blind).await
    }

    pub async fn sign_blinds(
        &self,
        blinds: &[cashu::BlindedMessage],
    ) -> Result<Vec<cashu::BlindSignature>> {
        let signatures = self.generate_signatures(blinds).await?;
        for (blind, signature) in blinds.iter().zip(signatures.iter()) {
            self.repository
                .signature_store(blind.blinded_secret, signature.clone())
                .await?;
        }
        Ok(signatures)
    }

    async fn generate_signatures(
        &self,
        blinds: &[cashu::BlindedMessage],
    ) -> Result<Vec<cashu::BlindSignature>> {
        let Some(first_blind) = blinds.first() else {
            return Ok(Vec::new());
        };
        let mut keyset = self.keys(first_blind.keyset_id).await?;
        let mut signatures = Vec::with_capacity(blinds.len());
        for blind in blinds {
            let current_keyset = if blind.keyset_id == keyset.id {
                &keyset
            } else {
                keyset = self.keys(blind.keyset_id).await?;
                &keyset
            };
            signatures.push(sign_ecash(current_keyset, blind)?);
        }
        Ok(signatures)
    }

    async fn list_kinfos(&self) -> Result<HashMap<cashu::Id, cashu::KeySetInfo>> {
        let kinfos = self.list_info(ListFilters::default()).await?;
        Ok(HashMap::from_iter(
            kinfos
                .into_iter()
                .map(|kinfo| (kinfo.id, cashu::KeySetInfo::from(kinfo))),
        ))
    }

    async fn get_keyset(&self, kid: cashu::Id) -> Result<cashu::KeySet> {
        let keyset = self.keys(kid).await?;
        Ok(bcr_wdc_utils::keys::to_keyset(&keyset, None))
    }

    pub async fn check_state(
        &self,
        ys: &[cashu::PublicKey],
        now: TStamp,
    ) -> Result<Vec<cashu::ProofState>> {
        self.repository.commitment_clean_expired(now).await?;
        self.repository.ys_clean_expired(now).await?;
        let joined_spent = ys
            .iter()
            .map(|y| self.repository.proofs_contains(*y))
            .collect::<JoinAll<_>>();
        let states: Vec<_> = joined_spent.await.into_iter().collect::<Result<_>>()?;
        let reserveds = self.repository.ys_contains(ys).await?;
        let mut proof_states = Vec::with_capacity(states.len());
        for (state, reserved, y) in izip!(states.into_iter(), reserveds.into_iter(), ys.iter()) {
            if let Some(state) = state {
                proof_states.push(state);
            } else if reserved
                || self
                    .repository
                    .commitment_contains_inputs(std::slice::from_ref(y))
                    .await?
            {
                proof_states.push(cashu::ProofState {
                    y: *y,
                    state: cashu::State::Reserved,
                    witness: None,
                });
            } else {
                proof_states.push(cashu::ProofState {
                    y: *y,
                    state: cashu::State::Unspent,
                    witness: None,
                });
            }
        }
        Ok(proof_states)
    }

    pub async fn signed_commit_to_swap(
        &self,
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
        let owner = self.clowder.verify_pk(&content.wallet_key).await?;
        let signature_owner = SignatureOwner::from(owner);
        if now < self.settle_window_deadline && !matches!(signature_owner, SignatureOwner::Beta) {
            return Err(Error::ServiceUnavailable);
        }
        self.commit_to_swap_inner(content, now, signature_owner)
            .await
    }

    pub async fn commit_to_swap(
        &self,
        request: wire_swap::SwapCommitmentRequest,
        now: TStamp,
    ) -> Result<(String, schnorr::Signature)> {
        if now < self.settle_window_deadline {
            return Err(Error::ServiceUnavailable);
        }
        self.commit_to_swap_inner(request, now, SignatureOwner::Unsigned)
            .await
    }

    async fn commit_to_swap_inner(
        &self,
        request: wire_swap::SwapCommitmentRequest,
        now: TStamp,
        signed: SignatureOwner,
    ) -> Result<(String, schnorr::Signature)> {
        let expiry =
            chrono::DateTime::from_timestamp(request.expiry as i64, 0).ok_or_else(|| {
                Error::InvalidInput(BRError::Generic(String::from("invalid expiry timestamp")))
            })?;
        if expiry < now {
            return Err(Error::InvalidInput(BRError::Generic(String::from(
                "commitment already expired",
            ))));
        }
        let expiry = expiry.min(now + self.max_expiry);
        let core_fps = request
            .inputs
            .inputs
            .iter()
            .map(|fp| ProofFingerprint::from(fp.clone()))
            .collect::<Vec<_>>();
        signatures_utils::basic_fingerprints_checks(&core_fps)?;
        signatures_utils::basic_blinds_checks(&request.outputs)?;
        self.clowder
            .authenticate_attestation(&self.alpha_id, &request.inputs)
            .await?;
        let kinfos = self.list_kinfos().await?;
        swap::mint::verify_commit(&core_fps, &request.outputs, &kinfos)?;
        let ys: Vec<cashu::PublicKey> = request.inputs.inputs.iter().map(|fp| fp.y).collect();
        if !self
            .check_state(&ys, now)
            .await?
            .iter()
            .all(|state| matches!(state.state, cashu::State::Unspent))
        {
            return Err(Error::InvalidInput(BRError::Generic(String::from(
                "One or more proofs are not unspent",
            ))));
        }
        self.verify_fingerprints(&core_fps).await?;
        let bs: Vec<cashu::PublicKey> = request
            .outputs
            .iter()
            .map(|blind| blind.blinded_secret)
            .collect();
        if self.repository.commitment_contains_outputs(&bs).await? {
            return Err(Error::InvalidInput(BRError::Generic(String::from(
                "blinded messages committed",
            ))));
        }
        let wallet_key = request.wallet_key;
        let fp_digest = request.inputs.attestation.fp_digest;
        let (content, commitment) = self.clowder.commit_to_swap(request).await?;
        match self
            .repository
            .commitment_store(
                ys,
                bs,
                expiry,
                wallet_key.into(),
                commitment,
                fp_digest,
                signed,
            )
            .await
        {
            Ok(()) | Err(Error::Conflict(_)) => Ok((content, commitment)),
            Err(error) => {
                tracing::error!("failed to store commitment: {error}");
                Err(error)
            }
        }
    }

    pub async fn swap(
        &self,
        inputs: Vec<cashu::Proof>,
        outputs: Vec<cashu::BlindedMessage>,
        commitment: schnorr::Signature,
        now: TStamp,
    ) -> Result<Vec<cashu::BlindSignature>> {
        signatures_utils::basic_proofs_checks(&inputs)?;
        signatures_utils::basic_blinds_checks(&outputs)?;
        let StoredCommitment {
            outputs: committed_outputs,
            expiration,
            fp_digest: committed_fp_digest,
            signed,
            ..
        } = self.repository.commitment_load(&commitment).await?;
        if now < self.settle_window_deadline && !matches!(signed, SignatureOwner::Beta) {
            return Err(Error::ServiceUnavailable);
        }
        if expiration < now {
            return Err(Error::InvalidInput(BRError::Generic(String::from(
                "commitment has expired",
            ))));
        }
        let input_fps = wire_attestation::project_to_fingerprints(&inputs)?;
        if wire_attestation::fp_digest(&input_fps) != committed_fp_digest {
            return Err(Error::Attestation(
                wire_attestation::AttestationError::DigestMismatch,
            ));
        }
        let output_bs: Vec<cashu::PublicKey> =
            outputs.iter().map(|blind| blind.blinded_secret).collect();
        if !cross_check_commits_swaps(&committed_outputs, &output_bs) {
            return Err(Error::InvalidInput(BRError::Generic(format!(
                "output/committed_outputs mismatch {:?}/{:?}",
                output_bs, committed_outputs,
            ))));
        }
        let (kinfos, _) = tokio::try_join!(self.list_kinfos(), self.verify_proofs(&inputs))?;
        let fee_policy = match signed {
            SignatureOwner::Alpha | SignatureOwner::Beta => swap::mint::FeePolicy::Ignore,
            SignatureOwner::Unsigned => swap::mint::FeePolicy::Apply,
        };
        swap::mint::verify_swap(&inputs, &outputs, &kinfos, fee_policy)?;

        let signatures = self.generate_signatures(&outputs).await?;
        let fee_premints = self.generate_fees_premints(&inputs, &outputs).await?;
        let fees = self.sign_fees(fee_premints).await?;
        self.clowder
            .signal_swap_event(
                inputs.clone(),
                outputs.clone(),
                fees.signatures.clone(),
                commitment,
                signatures.clone(),
            )
            .await?;

        let mut stored_signatures = outputs
            .iter()
            .zip(signatures.iter())
            .map(|(blind, signature)| StoredSignature {
                y: blind.blinded_secret,
                signature: signature.clone(),
            })
            .collect::<Vec<_>>();
        stored_signatures.extend(fees.stored_signatures);
        self.repository
            .swap_finalize(inputs, stored_signatures, commitment)
            .await?;
        self.treasury.store_proofs(fees.proofs).await?;
        Ok(signatures)
    }

    async fn generate_fees_premints(
        &self,
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
                .filter(|blind| blind.keyset_id == kid)
                .fold(cashu::Amount::ZERO, |acc, blind| acc + blind.amount);
            if inputs_amount <= outputs_amount {
                continue;
            }
            let keyset = self.get_keyset(kid).await?;
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

    async fn sign_fees(&self, premints: Vec<cashu::PreMintSecrets>) -> Result<GeneratedFees> {
        let total_len = premints.iter().map(cashu::PreMintSecrets::len).sum();
        let mut generated = GeneratedFees {
            signatures: Vec::with_capacity(total_len),
            proofs: Vec::with_capacity(total_len),
            stored_signatures: Vec::with_capacity(total_len),
        };
        for premint in premints {
            let keyset = self.get_keyset(premint.keyset_id).await?;
            let blinded_messages = premint.blinded_messages();
            let signatures = self.generate_signatures(&blinded_messages).await?;
            generated
                .stored_signatures
                .extend(blinded_messages.iter().zip(signatures.iter()).map(
                    |(blind, signature)| StoredSignature {
                        y: blind.blinded_secret,
                        signature: signature.clone(),
                    },
                ));
            let (rs, secrets) = premint
                .secrets
                .into_iter()
                .map(|premint| (premint.r, premint.secret))
                .unzip();
            let proofs =
                cashu::dhke::construct_proofs(signatures.clone(), rs, secrets, &keyset.keys)?;
            generated.signatures.extend(signatures);
            generated.proofs.extend(proofs);
        }
        Ok(generated)
    }

    pub async fn burn(&self, proofs: Vec<cashu::Proof>) -> Result<Vec<cashu::PublicKey>> {
        signatures_utils::basic_proofs_checks(&proofs)?;
        self.verify_proofs(&proofs).await?;
        let mut ys = Vec::with_capacity(proofs.len());
        for proof in &proofs {
            ys.push(cashu::dhke::hash_to_curve(proof.secret.as_bytes())?);
        }
        self.repository.proofs_insert(proofs).await?;
        Ok(ys)
    }

    pub async fn recover(&self, proofs: &[cashu::Proof]) -> Result<()> {
        let ys = proofs
            .iter()
            .map(|proof| cashu::dhke::hash_to_curve(proof.secret.as_bytes()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.repository.proofs_remove(&ys).await?;
        Ok(())
    }

    pub async fn reserve(&self, ys: Vec<cashu::PublicKey>, deadline: TStamp) -> Result<()> {
        self.repository.ys_store(ys, deadline).await
    }
}

struct GeneratedFees {
    signatures: Vec<cashu::BlindSignature>,
    proofs: Vec<cashu::Proof>,
    stored_signatures: Vec<StoredSignature>,
}

fn cross_check_commits_swaps<T: PartialEq>(committed: &[T], swap: &[T]) -> bool {
    committed.len() == swap.len()
        && committed
            .iter()
            .all(|committed| swap.iter().any(|item| item == committed))
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bcr_common::core_tests;
    use bcr_wdc_utils::signatures::test_utils as signatures_test;
    use bitcoin::bip32::DerivationPath;

    use super::*;
    use crate::{clients::MockClowderClient, persistence::inmemory, swap::MockTreasuryService};

    fn seed() -> [u8; 64] {
        bip39::Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap()
        .to_seed("")
    }

    async fn prepare_swap(
        repository: &inmemory::Repository,
    ) -> (
        cashu::MintKeySet,
        Vec<cashu::Proof>,
        Vec<cashu::BlindedMessage>,
        schnorr::Signature,
        TStamp,
    ) {
        let (mut kinfo, mut keyset) = core_tests::generate_random_ecash_keyset();
        kinfo.input_fee_ppk = 0;
        keyset.input_fee_ppk = 0;
        repository
            .keys_store((kinfo, keyset.clone()))
            .await
            .unwrap();
        let amounts = [cashu::Amount::from(8u64)];
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        let outputs = signatures_test::generate_blinds(keyset.id, &amounts)
            .into_iter()
            .map(|generated| generated.0)
            .collect::<Vec<_>>();
        let commitment = schnorr::Signature::from_slice(&[17u8; 64]).unwrap();
        let now = chrono::Utc::now();
        let fp_digest = wire_attestation::fp_digest(
            &wire_attestation::project_to_fingerprints(&proofs).unwrap(),
        );
        repository
            .commitment_store(
                proofs.iter().map(|proof| proof.y().unwrap()).collect(),
                outputs.iter().map(|blind| blind.blinded_secret).collect(),
                now + chrono::Duration::minutes(1),
                bcr_common::core::generate_random_keypair()
                    .public_key()
                    .into(),
                commitment,
                fp_digest,
                SignatureOwner::Alpha,
            )
            .await
            .unwrap();
        (keyset, proofs, outputs, commitment, now)
    }

    fn service(
        repository: Arc<inmemory::Repository>,
        clowder: MockClowderClient,
        treasury: MockTreasuryService,
    ) -> Service {
        Service {
            repository,
            clowder: Box::new(clowder),
            treasury: Box::new(treasury),
            keygen: Factory::new(&seed(), DerivationPath::default()),
            min_keyset_fees_ppk: AtomicU64::default(),
            max_expiry: chrono::Duration::hours(1),
            alpha_id: bcr_common::core::generate_random_keypair().public_key(),
            settle_window_deadline: TStamp::default(),
        }
    }

    #[tokio::test]
    async fn swap_atomically_persists_proofs_signatures_and_commitment_deletion() {
        let repository = Arc::new(inmemory::Repository::default());
        let (_, proofs, outputs, commitment, now) = prepare_swap(&repository).await;
        let mut clowder = MockClowderClient::new();
        clowder
            .expect_signal_swap_event()
            .times(1)
            .returning(|_, _, _, _, _| Ok(()));
        let mut treasury = MockTreasuryService::new();
        treasury
            .expect_store_proofs()
            .times(1)
            .returning(|_| Ok(()));
        let service = service(repository.clone(), clowder, treasury);

        let signatures = service
            .swap(proofs.clone(), outputs.clone(), commitment, now)
            .await
            .unwrap();

        assert_eq!(signatures.len(), outputs.len());
        assert!(repository
            .proofs_contains(proofs[0].y().unwrap())
            .await
            .unwrap()
            .is_some());
        assert_eq!(
            repository.signature_load(&outputs[0]).await.unwrap(),
            Some(signatures[0].clone())
        );
        assert!(matches!(
            repository.commitment_load(&commitment).await,
            Err(Error::ResourceNotFound(_))
        ));
    }

    #[tokio::test]
    async fn swap_does_not_store_signatures_when_proof_insertion_fails() {
        let repository = Arc::new(inmemory::Repository::default());
        let (_, proofs, outputs, commitment, now) = prepare_swap(&repository).await;
        repository.proofs_insert(proofs.clone()).await.unwrap();
        let mut clowder = MockClowderClient::new();
        clowder
            .expect_signal_swap_event()
            .times(1)
            .returning(|_, _, _, _, _| Ok(()));
        let mut treasury = MockTreasuryService::new();
        treasury.expect_store_proofs().times(0);
        let service = service(repository.clone(), clowder, treasury);

        let result = service.swap(proofs, outputs.clone(), commitment, now).await;
        assert!(matches!(result, Err(Error::Conflict(_))));

        assert_eq!(repository.signature_load(&outputs[0]).await.unwrap(), None);
        assert!(repository.commitment_load(&commitment).await.is_ok());
    }
}
