// ----- standard library imports
use std::{
    collections::{BTreeSet, HashMap},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
// ----- extra library imports
use bcr_common::{
    cashu,
    client::admin::core::{BRError, RNFError},
    core::{
        keys as core_keys, maturity,
        signature::{
            self, sign_ecash, verify_ecash_fingerprint, verify_ecash_proof, ProofFingerprint,
        },
        swap,
    },
    ecash,
    wire::{attestation as wire_attestation, swap as wire_swap},
};
use bcr_wdc_utils::{keys as keys_utils, signatures as signatures_utils};
use bitcoin::secp256k1::PublicKey;
use futures::future::JoinAll;
use itertools::izip;
use secp256k1::schnorr;
// ----- local imports
use crate::{
    clients::{ClowderClient, TreasuryService},
    error::{Error, Result},
    factory::Factory,
    persistence::{Repository, SignatureOwner, StoredCommitment, StoredSignature},
    TStamp,
};

// ----- end imports

#[derive(Default)]
pub struct ListFilters {
    pub unit: Option<cashu::CurrencyUnit>,
    pub min_expiration: Option<time::Date>,
    pub max_expiration: Option<time::Date>,
}

pub struct Service {
    pub repository: Arc<dyn Repository>,
    pub clowder: Box<dyn ClowderClient>,
    pub treasury: Box<dyn TreasuryService>,
    pub keygen: Factory,
    pub min_keyset_fees_ppk: AtomicU64,
    pub max_expiry: time::Duration,
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
        expiration: Option<u64>,
        fees_ppk: u64,
    ) -> Result<ecash::MintKeySetInfo> {
        let fees_ppk = std::cmp::max(fees_ppk, self.min_keyset_fees_ppk.load(Ordering::Relaxed));
        let entry = self.keygen.generate(unit, now, expiration, fees_ppk);
        let (kinfo, keyset) = bcr_wdc_utils::keys::from_entry(entry.clone());
        self.clowder.new_keyset(ecash::KeySet::from(keyset)).await?;
        self.repository.keys_store(entry).await?;
        Ok(kinfo)
    }

    pub async fn info(&self, kid: cashu::Id) -> Result<ecash::MintKeySetInfo> {
        self.repository
            .keys_info(kid)
            .await?
            .ok_or(Error::ResourceNotFound(RNFError::KeysetId(kid)))
    }

    pub async fn keys(&self, kid: cashu::Id) -> Result<ecash::MintKeySet> {
        self.repository
            .keys_load(kid)
            .await?
            .ok_or(Error::ResourceNotFound(RNFError::KeysetId(kid)))
    }

    pub async fn verify_proofs(&self, proofs: &[cashu::Proof]) -> Result<()> {
        let by_kid = bcr_common::core::signature::proofs_to_map(proofs.iter().cloned());
        for (kid, proofs) in by_kid {
            let keyset = self.keys(kid).await?;
            let c_keyset = cashu::MintKeySet::from(keyset);
            for proof in &proofs {
                verify_ecash_proof(&c_keyset, proof)?;
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
            let c_keyset = cashu::MintKeySet::from(keyset);
            for fp in fps {
                verify_ecash_fingerprint(&c_keyset, fp)?;
            }
        }
        Ok(())
    }

    pub async fn list_info(&self, filters: ListFilters) -> Result<Vec<ecash::MintKeySetInfo>> {
        let min_tstamp = filters.min_expiration.map(maturity::credit_expires_at);
        let max_tstamp = filters.max_expiration.map(maturity::credit_expires_at);
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
            let current_keyset = if blind.keyset_id == keyset.id.into() {
                &keyset
            } else {
                keyset = self.keys(blind.keyset_id).await?;
                &keyset
            };
            signatures.push(sign_ecash(current_keyset, blind)?);
        }
        Ok(signatures)
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
            time::OffsetDateTime::from_unix_timestamp(request.expiry as i64).map_err(|_| {
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
        let c_outputs: Vec<_> = request.outputs.iter().cloned().map(From::from).collect();
        signatures_utils::basic_blinds_checks(&c_outputs)?;
        self.clowder
            .authenticate_attestation(&self.alpha_id, &request.inputs)
            .await?;
        let kinfos = self.list_info(ListFilters::default()).await?;
        let kinfos = keys_utils::kinfos_list_to_map(kinfos);
        let kinfos = kinfos.into_iter().collect::<HashMap<_, _>>();
        swap::mint::verify_commit(&core_fps, &c_outputs, &kinfos)?;
        let ys: Vec<cashu::PublicKey> = request.inputs.inputs.iter().map(|fp| fp.y).collect();
        for state in self.check_state(&ys, now).await? {
            match state.state {
                // Reserved inputs are not rejected here: `reserve` below fails on a genuine
                // conflict, and that failure path already recovers an identical retry.
                cashu::State::Unspent | cashu::State::Reserved => {}
                _ => {
                    return Err(Error::InvalidInput(BRError::Generic(String::from(
                        "One or more proofs are not unspent",
                    ))));
                }
            }
        }
        self.verify_fingerprints(&core_fps).await?;
        let bs: Vec<cashu::PublicKey> = request
            .outputs
            .iter()
            .map(|blind| blind.blinded_secret)
            .collect();
        let wallet_key: cashu::PublicKey = request.wallet_key.into();
        // An identical retry (same inputs, outputs, expiry and wallet key) returns the
        // original commitment instead of tripping the conflict checks below.
        if let Some(signature) = self
            .find_identical_commitment(&ys, &bs, expiry, wallet_key)
            .await?
        {
            return Ok((reencode_commit_request(&request)?, signature));
        }
        if self.repository.commitment_contains_outputs(&bs).await? {
            return Err(Error::InvalidInput(BRError::Generic(String::from(
                "blinded messages committed",
            ))));
        }
        // Reserve the inputs before Clowder is asked to sign: a concurrent request over an
        // overlapping set of ys fails here instead of racing this one to commitment_store.
        // The deadline mirrors the commitment's own expiry, so a request that fails after this
        // point releases its reservation the same way check_state already reaps expired ones.
        if let Err(error) = self.reserve(ys.clone(), expiry).await {
            return match self
                .find_identical_commitment(&ys, &bs, expiry, wallet_key)
                .await?
            {
                Some(signature) => Ok((reencode_commit_request(&request)?, signature)),
                None => Err(error),
            };
        }
        let fp_digest = request.inputs.attestation.fp_digest;
        let (content, commitment) = self.clowder.commit_to_swap(request).await?;
        match self
            .repository
            .commitment_store(
                ys.clone(),
                bs.clone(),
                expiry,
                wallet_key,
                commitment,
                fp_digest,
                signed,
            )
            .await
        {
            Ok(()) => Ok((content, commitment)),
            Err(error @ Error::Conflict(_)) => {
                match self
                    .find_identical_commitment(&ys, &bs, expiry, wallet_key)
                    .await?
                {
                    Some(signature) => Ok((content, signature)),
                    None => {
                        tracing::error!("failed to store commitment: {error}");
                        Err(error)
                    }
                }
            }
            Err(error) => {
                tracing::error!("failed to store commitment: {error}");
                Err(error)
            }
        }
    }

    /// On a request identical to one already committed (same inputs, outputs, expiry and
    /// wallet key), returns that commitment instead of a fresh `Conflict`: a wallet retrying
    /// a timed-out or lost response must get its original commitment back, not an error.
    async fn find_identical_commitment(
        &self,
        ys: &[cashu::PublicKey],
        bs: &[cashu::PublicKey],
        expiry: TStamp,
        wallet_key: cashu::PublicKey,
    ) -> Result<Option<schnorr::Signature>> {
        let Some(y) = ys.first() else {
            return Ok(None);
        };
        let Some(signature) = self.repository.commitment_find_by_input(*y).await? else {
            return Ok(None);
        };
        let stored = self.repository.commitment_load(&signature).await?;
        let mut stored_inputs = stored.inputs;
        stored_inputs.sort();
        let mut requested_inputs = ys.to_vec();
        requested_inputs.sort();
        let mut stored_outputs = stored.outputs;
        stored_outputs.sort();
        let mut requested_outputs = bs.to_vec();
        requested_outputs.sort();
        if stored_inputs == requested_inputs
            && stored_outputs == requested_outputs
            && stored.expiration == expiry
            && stored.wallet_key == wallet_key
        {
            Ok(Some(signature))
        } else {
            Ok(None)
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
        let stored_commitment = match self.repository.commitment_load(&commitment).await {
            Ok(stored) => stored,
            Err(error @ Error::ResourceNotFound(_)) => {
                // The commitment is gone exactly when `swap_finalize` has already run for it
                // (it deletes the commitment on success). If these inputs are already spent and
                // the client's own outputs and this commitment's fee outputs already carry a
                // signature, this is a retry of a swap that already finalized but whose reply or
                // Clowder signal was lost: signal Clowder again with the original fees and replay
                // the stored signatures rather than fail a swap that already happened.
                let Some((fees, signatures)) = self
                    .recover_finalized_swap(&inputs, &outputs, &commitment)
                    .await?
                else {
                    return Err(error);
                };
                self.clowder
                    .signal_swap_event(inputs, outputs, fees, commitment, signatures.clone())
                    .await?;
                tracing::info!(
                    "replayed stored signatures for a finalized swap retry, commitment {commitment}"
                );
                return Ok(signatures);
            }
            Err(error) => return Err(error),
        };
        let StoredCommitment {
            outputs: committed_outputs,
            expiration,
            fp_digest: committed_fp_digest,
            signed,
            ..
        } = stored_commitment;
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
        let (kinfos, _) = tokio::try_join!(
            self.list_info(ListFilters::default()),
            self.verify_proofs(&inputs)
        )?;
        let fee_policy = match signed {
            SignatureOwner::Alpha | SignatureOwner::Beta => swap::mint::FeePolicy::Ignore,
            SignatureOwner::Unsigned => swap::mint::FeePolicy::Apply,
        };
        let kinfo = keys_utils::kinfos_list_to_map(kinfos.clone());
        let kinfos = kinfo.into_iter().collect::<HashMap<_, _>>();
        swap::mint::verify_swap(&inputs, &outputs, &kinfos, fee_policy)?;
        let signatures = self.generate_signatures(&outputs).await?;
        let fee_premints = self
            .generate_fees_premints(&inputs, &outputs, &commitment)
            .await?;
        let fees = self.sign_fees(fee_premints).await?;

        let mut stored_signatures = outputs
            .iter()
            .zip(signatures.iter())
            .map(|(blind, signature)| StoredSignature {
                y: blind.blinded_secret,
                signature: signature.clone(),
            })
            .collect::<Vec<_>>();
        stored_signatures.extend(fees.stored_signatures);
        // Persist the swap as spent before telling Clowder about it: a failed store must fail
        // the request, not notify Clowder of a spend that was never recorded.
        let notified_inputs = inputs.clone();
        let notified_outputs = outputs.clone();
        self.repository
            .swap_finalize(inputs, stored_signatures, commitment)
            .await?;
        self.treasury.store_proofs(fees.proofs).await?;
        self.clowder
            .signal_swap_event(
                notified_inputs,
                notified_outputs,
                fees.signatures.clone(),
                commitment,
                signatures.clone(),
            )
            .await?;
        Ok(signatures)
    }

    /// `Some((fees, signatures))` when every input is already spent and every requested output
    /// and every fee output derived from `commitment` already carries a stored signature: i.e.
    /// this exact swap already finalized. `None` otherwise, meaning the missing commitment is a
    /// genuine error rather than a retry.
    async fn recover_finalized_swap(
        &self,
        inputs: &[cashu::Proof],
        outputs: &[cashu::BlindedMessage],
        commitment: &schnorr::Signature,
    ) -> Result<Option<(Vec<cashu::BlindSignature>, Vec<cashu::BlindSignature>)>> {
        for proof in inputs {
            let y = proof.y()?;
            match self.repository.proofs_contains(y).await? {
                Some(state) if matches!(state.state, cashu::State::Spent) => {}
                _ => return Ok(None),
            }
        }
        let Some(signatures) = self.load_signatures(outputs).await? else {
            return Ok(None);
        };
        let fee_blinds = self
            .generate_fees_premints(inputs, outputs, commitment)
            .await?
            .iter()
            .flat_map(cashu::PreMintSecrets::blinded_messages)
            .collect::<Vec<_>>();
        let Some(fees) = self.load_signatures(&fee_blinds).await? else {
            return Ok(None);
        };
        Ok(Some((fees, signatures)))
    }

    async fn load_signatures(
        &self,
        blinds: &[cashu::BlindedMessage],
    ) -> Result<Option<Vec<cashu::BlindSignature>>> {
        let mut signatures = Vec::with_capacity(blinds.len());
        for blind in blinds {
            match self.repository.signature_load(blind).await? {
                Some(signature) => signatures.push(signature),
                None => return Ok(None),
            }
        }
        Ok(Some(signatures))
    }

    /// Fee outputs are derived from `commitment`, so a retry of the same swap regenerates
    /// the same blinded messages, in the same order.
    async fn generate_fees_premints(
        &self,
        inputs: &[cashu::Proof],
        outputs: &[cashu::BlindedMessage],
        commitment: &schnorr::Signature,
    ) -> Result<Vec<cashu::PreMintSecrets>> {
        let unique_kids: BTreeSet<_> = inputs.iter().map(|proof| proof.keyset_id).collect();
        let seed = self.keygen.swap_fees_seed(commitment);
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
            let keyset = self.keys(kid).await?;
            let c_keyset = core_keys::to_keyset(&keyset, None);
            let premint = cashu::PreMintSecrets::from_seed(
                kid,
                0,
                &seed,
                inputs_amount - outputs_amount,
                &cashu::amount::SplitTarget::None,
                &bcr_wdc_utils::keys::to_fee_and_amounts(&c_keyset),
            )
            .map_err(|e| Error::Internal(format!("failed to derive fee outputs: {e}")))?;
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
            let keyset = self.keys(premint.keyset_id).await?;
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
            let c_keys = core_keys::to_keyset(&keyset, None).keys;
            let proofs = cashu::dhke::construct_proofs(signatures.clone(), rs, secrets, &c_keys)?;
            generated.signatures.extend(signatures);
            generated.proofs.extend(proofs);
        }
        Ok(generated)
    }

    pub async fn burn(&self, proofs: Vec<cashu::Proof>) -> Result<Vec<cashu::PublicKey>> {
        let fps: Vec<ProofFingerprint> = wire_attestation::project_to_fingerprints(&proofs)?
            .into_iter()
            .map(ProofFingerprint::from)
            .collect();
        signatures_utils::basic_fingerprints_checks(&fps)?;
        self.verify_fingerprints(&fps).await?;
        self.repository.proofs_insert(proofs).await?;
        Ok(fps.into_iter().map(|fp| fp.y.into()).collect())
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

fn reencode_commit_request(request: &wire_swap::SwapCommitmentRequest) -> Result<String> {
    let (content, _) = signature::serialize_borsh_msg_b64(request)
        .map_err(|e| Error::Internal(format!("failed to serialize commitment: {e}")))?;
    Ok(content)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bcr_common::{core_tests, wire::keys as wire_keys};
    use bcr_wdc_utils::signatures::test_utils as signatures_test;
    use bitcoin::{
        bip32::DerivationPath,
        hashes::{sha256::Hash as Sha256Hash, Hash},
    };

    use super::*;
    use crate::{
        clients::{MockClowderClient, MockTreasuryService},
        persistence::inmemory,
    };

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
        ecash::MintKeySet,
        Vec<cashu::Proof>,
        Vec<cashu::BlindedMessage>,
        schnorr::Signature,
        TStamp,
    ) {
        let amounts = [cashu::Amount::from(8u64)];
        prepare_swap_with(repository, &amounts, &amounts, SignatureOwner::Alpha).await
    }

    async fn prepare_swap_with(
        repository: &inmemory::Repository,
        input_amounts: &[cashu::Amount],
        output_amounts: &[cashu::Amount],
        signed: SignatureOwner,
    ) -> (
        ecash::MintKeySet,
        Vec<cashu::Proof>,
        Vec<cashu::BlindedMessage>,
        schnorr::Signature,
        TStamp,
    ) {
        let (mut kinfo, mut keyset) = core_tests::generate_random_ecash_keyset();
        kinfo.input_fee_ppk = 0;
        keyset.input_fee_ppk = 0;
        let entry = keys_utils::to_entry(kinfo, keyset.clone());
        repository.keys_store(entry).await.unwrap();
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, input_amounts);
        let outputs = signatures_test::generate_blinds(keyset.id.into(), output_amounts)
            .into_iter()
            .map(|generated| generated.0)
            .collect::<Vec<_>>();
        let commitment = schnorr::Signature::from_slice(&[17u8; 64]).unwrap();
        let now = time::OffsetDateTime::now_utc();
        let fp_digest = wire_attestation::fp_digest(
            &wire_attestation::project_to_fingerprints(&proofs).unwrap(),
        );
        repository
            .commitment_store(
                proofs.iter().map(|proof| proof.y().unwrap()).collect(),
                outputs.iter().map(|blind| blind.blinded_secret).collect(),
                now + time::Duration::minutes(1),
                bcr_common::core::generate_random_keypair()
                    .public_key()
                    .into(),
                commitment,
                fp_digest,
                signed,
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
            max_expiry: time::Duration::hours(1),
            alpha_id: bcr_common::core::generate_random_keypair().public_key(),
            settle_window_deadline: TStamp::UNIX_EPOCH,
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
    async fn swap_ordering_does_not_signal_clowder_when_persist_fails() {
        let repository = Arc::new(inmemory::Repository::default());
        let (_, proofs, outputs, commitment, now) = prepare_swap(&repository).await;
        repository.proofs_insert(proofs.clone()).await.unwrap();
        let mut clowder = MockClowderClient::new();
        clowder.expect_signal_swap_event().times(0);
        let mut treasury = MockTreasuryService::new();
        treasury.expect_store_proofs().times(0);
        let service = service(repository.clone(), clowder, treasury);

        let result = service.swap(proofs, outputs.clone(), commitment, now).await;
        assert!(matches!(result, Err(Error::Conflict(_))));

        assert_eq!(repository.signature_load(&outputs[0]).await.unwrap(), None);
        assert!(repository.commitment_load(&commitment).await.is_ok());
    }

    #[tokio::test]
    async fn swap_ordering_rejects_overlapping_outputs_via_commitment_store() {
        let repository = inmemory::Repository::default();
        let (mut kinfo, mut keyset) = core_tests::generate_random_ecash_keyset();
        kinfo.input_fee_ppk = 0;
        keyset.input_fee_ppk = 0;
        let amounts = [cashu::Amount::from(8u64)];
        let proofs_a = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        let proofs_b = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        let outputs: Vec<_> = signatures_test::generate_blinds(keyset.id.into(), &amounts)
            .into_iter()
            .map(|generated| generated.0.blinded_secret)
            .collect();
        let now = time::OffsetDateTime::now_utc();
        let expiry = now + time::Duration::minutes(1);
        let wallet_key = bcr_common::core::generate_random_keypair()
            .public_key()
            .into();
        let commitment_a = schnorr::Signature::from_slice(&[11u8; 64]).unwrap();
        let commitment_b = schnorr::Signature::from_slice(&[12u8; 64]).unwrap();

        repository
            .commitment_store(
                proofs_a.iter().map(|proof| proof.y().unwrap()).collect(),
                outputs.clone(),
                expiry,
                wallet_key,
                commitment_a,
                [0u8; 32],
                SignatureOwner::Unsigned,
            )
            .await
            .expect("first commitment over these outputs must succeed");

        let result = repository
            .commitment_store(
                proofs_b.iter().map(|proof| proof.y().unwrap()).collect(),
                outputs,
                expiry,
                wallet_key,
                commitment_b,
                [0u8; 32],
                SignatureOwner::Unsigned,
            )
            .await;

        assert!(
            matches!(result, Err(Error::Conflict(_))),
            "a second commitment over already-committed outputs (disjoint inputs) must fail atomically, got {result:?}"
        );
    }

    #[tokio::test]
    async fn swap_ordering_rejects_overlapping_inputs_before_calling_clowder() {
        let repository = Arc::new(inmemory::Repository::default());
        let (kinfo, keyset) = core_tests::generate_random_ecash_keyset();
        repository
            .keys_store(keys_utils::to_entry(kinfo, keyset.clone()))
            .await
            .unwrap();
        let amounts = [cashu::Amount::from(8u64)];
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        let proof_fps: Vec<wire_keys::ProofFingerprint> = proofs
            .iter()
            .cloned()
            .map(wire_keys::ProofFingerprint::try_from)
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        let now = time::OffsetDateTime::now_utc();
        let expiry = (now + time::Duration::minutes(2)).unix_timestamp() as u64;

        let mut clowder = MockClowderClient::new();
        clowder
            .expect_authenticate_attestation()
            .returning(|_, _| Ok(()));
        clowder.expect_commit_to_swap().times(1).returning(|_| {
            Ok((
                String::new(),
                schnorr::Signature::from_slice(&[9u8; 64]).unwrap(),
            ))
        });
        let service = service(repository.clone(), clowder, MockTreasuryService::new());

        let blinds_a: Vec<_> = signatures_test::generate_blinds(keyset.id.into(), &amounts)
            .into_iter()
            .map(|generated| generated.0)
            .collect();
        let request_a = wire_swap::SwapCommitmentRequest {
            inputs: crate::test_utils::attested_fingerprints(proof_fps.clone()),
            outputs: blinds_a.iter().cloned().map(From::from).collect(),
            expiry,
            wallet_key: bcr_common::core::generate_random_keypair().public_key(),
        };
        service
            .commit_to_swap(request_a, now)
            .await
            .expect("first commitment over these inputs must succeed");

        let blinds_b: Vec<_> = signatures_test::generate_blinds(keyset.id.into(), &amounts)
            .into_iter()
            .map(|generated| generated.0)
            .collect();
        let request_b = wire_swap::SwapCommitmentRequest {
            inputs: crate::test_utils::attested_fingerprints(proof_fps),
            outputs: blinds_b.iter().cloned().map(From::from).collect(),
            expiry,
            wallet_key: bcr_common::core::generate_random_keypair().public_key(),
        };
        let result = service.commit_to_swap(request_b, now).await;

        assert!(
            matches!(result, Err(Error::Conflict(_))),
            "a second commitment over already-reserved inputs must fail before Clowder signs it again, got {result:?}"
        );
    }

    #[tokio::test]
    async fn swap_ordering_reservation_expires_after_failed_commit() {
        let repository = Arc::new(inmemory::Repository::default());
        let (kinfo, keyset) = core_tests::generate_random_ecash_keyset();
        repository
            .keys_store(keys_utils::to_entry(kinfo, keyset.clone()))
            .await
            .unwrap();
        let amounts = [cashu::Amount::from(8u64)];
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        let proof_fps: Vec<wire_keys::ProofFingerprint> = proofs
            .iter()
            .cloned()
            .map(wire_keys::ProofFingerprint::try_from)
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        let blinds: Vec<_> = signatures_test::generate_blinds(keyset.id.into(), &amounts)
            .into_iter()
            .map(|generated| generated.0)
            .collect();
        let now = time::OffsetDateTime::now_utc();
        let expiry = (now + time::Duration::seconds(30)).unix_timestamp() as u64;

        let mut clowder = MockClowderClient::new();
        clowder
            .expect_authenticate_attestation()
            .returning(|_, _| Ok(()));
        clowder
            .expect_commit_to_swap()
            .times(1)
            .returning(|_| Err(Error::ServiceUnavailable));
        let service = service(repository.clone(), clowder, MockTreasuryService::new());

        let request = wire_swap::SwapCommitmentRequest {
            inputs: crate::test_utils::attested_fingerprints(proof_fps),
            outputs: blinds.iter().cloned().map(From::from).collect(),
            expiry,
            wallet_key: bcr_common::core::generate_random_keypair().public_key(),
        };
        let ys: Vec<_> = proofs.iter().map(|proof| proof.y().unwrap()).collect();

        let result = service.commit_to_swap(request, now).await;
        assert!(result.is_err(), "Clowder failing must fail the request");
        assert_eq!(
            repository.ys_contains(&ys).await.unwrap(),
            vec![true],
            "the failed commit's inputs must stay reserved until the commitment's own expiry"
        );

        let after_expiry = time::OffsetDateTime::from_unix_timestamp(expiry as i64).unwrap()
            + time::Duration::seconds(1);
        repository.ys_clean_expired(after_expiry).await.unwrap();
        assert_eq!(
            repository.ys_contains(&ys).await.unwrap(),
            vec![false],
            "the reservation must release once the commitment's own expiry has passed"
        );
    }

    #[tokio::test]
    async fn swap_ordering_identical_commit_retry_returns_stored_commitment() {
        let repository = Arc::new(inmemory::Repository::default());
        let (kinfo, keyset) = core_tests::generate_random_ecash_keyset();
        repository
            .keys_store(keys_utils::to_entry(kinfo, keyset.clone()))
            .await
            .unwrap();
        let amounts = [cashu::Amount::from(8u64)];
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        let proof_fps: Vec<wire_keys::ProofFingerprint> = proofs
            .iter()
            .cloned()
            .map(wire_keys::ProofFingerprint::try_from)
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        let blinds: Vec<_> = signatures_test::generate_blinds(keyset.id.into(), &amounts)
            .into_iter()
            .map(|generated| generated.0)
            .collect();
        let now = time::OffsetDateTime::now_utc();
        let expiry = (now + time::Duration::minutes(2)).unix_timestamp() as u64;

        let mut clowder = MockClowderClient::new();
        clowder
            .expect_authenticate_attestation()
            .returning(|_, _| Ok(()));
        // The real ClowderCl implementation's `content` is always the request's own canonical
        // encoding (see ClowderCl::commit_to_swap), so the mock mirrors that here: a retry must
        // see the same bytes whether they come from Clowder or from the local dedup path.
        clowder
            .expect_commit_to_swap()
            .times(1)
            .returning(|request| {
                Ok((
                    reencode_commit_request(&request)?,
                    schnorr::Signature::from_slice(&[9u8; 64]).unwrap(),
                ))
            });
        let service = service(repository.clone(), clowder, MockTreasuryService::new());

        let request = wire_swap::SwapCommitmentRequest {
            inputs: crate::test_utils::attested_fingerprints(proof_fps),
            outputs: blinds.iter().cloned().map(From::from).collect(),
            expiry,
            wallet_key: bcr_common::core::generate_random_keypair().public_key(),
        };

        let (content_a, commitment_a) = service
            .commit_to_swap(request.clone(), now)
            .await
            .expect("the first request must succeed");

        let (content_b, commitment_b) = service
            .commit_to_swap(request, now)
            .await
            .expect("an identical retry must return the original commitment, not Conflict");

        assert_eq!(commitment_a, commitment_b);
        assert_eq!(content_a, content_b);
    }

    #[tokio::test]
    async fn swap_ordering_different_commit_retry_over_same_inputs_conflicts() {
        let repository = Arc::new(inmemory::Repository::default());
        let (kinfo, keyset) = core_tests::generate_random_ecash_keyset();
        repository
            .keys_store(keys_utils::to_entry(kinfo, keyset.clone()))
            .await
            .unwrap();
        let amounts = [cashu::Amount::from(8u64)];
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        let proof_fps: Vec<wire_keys::ProofFingerprint> = proofs
            .iter()
            .cloned()
            .map(wire_keys::ProofFingerprint::try_from)
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        let now = time::OffsetDateTime::now_utc();
        let expiry = (now + time::Duration::minutes(2)).unix_timestamp() as u64;

        let mut clowder = MockClowderClient::new();
        clowder
            .expect_authenticate_attestation()
            .returning(|_, _| Ok(()));
        clowder.expect_commit_to_swap().times(1).returning(|_| {
            Ok((
                String::new(),
                schnorr::Signature::from_slice(&[9u8; 64]).unwrap(),
            ))
        });
        let service = service(repository.clone(), clowder, MockTreasuryService::new());

        let blinds_a: Vec<_> = signatures_test::generate_blinds(keyset.id.into(), &amounts)
            .into_iter()
            .map(|generated| generated.0)
            .collect();
        let request_a = wire_swap::SwapCommitmentRequest {
            inputs: crate::test_utils::attested_fingerprints(proof_fps.clone()),
            outputs: blinds_a.iter().cloned().map(From::from).collect(),
            expiry,
            wallet_key: bcr_common::core::generate_random_keypair().public_key(),
        };
        service
            .commit_to_swap(request_a, now)
            .await
            .expect("the first request must succeed");

        let blinds_b: Vec<_> = signatures_test::generate_blinds(keyset.id.into(), &amounts)
            .into_iter()
            .map(|generated| generated.0)
            .collect();
        let request_b = wire_swap::SwapCommitmentRequest {
            inputs: crate::test_utils::attested_fingerprints(proof_fps),
            outputs: blinds_b.iter().cloned().map(From::from).collect(),
            expiry,
            wallet_key: bcr_common::core::generate_random_keypair().public_key(),
        };
        let result = service.commit_to_swap(request_b, now).await;

        assert!(
            matches!(result, Err(Error::Conflict(_))),
            "a different request over the same already-committed inputs must fail, got {result:?}"
        );
    }

    type SignalledSwap = (
        Vec<cashu::BlindSignature>,
        schnorr::Signature,
        Vec<cashu::BlindSignature>,
    );

    #[tokio::test]
    async fn swap_retry_after_notify_failure_resignals_clowder_with_original_fees() {
        let repository = Arc::new(inmemory::Repository::default());
        let (_, proofs, outputs, commitment, now) = prepare_swap_with(
            &repository,
            &[cashu::Amount::from(16u64)],
            &[
                cashu::Amount::from(8u64),
                cashu::Amount::from(4u64),
                cashu::Amount::from(2u64),
            ],
            SignatureOwner::Unsigned,
        )
        .await;
        let signalled: Arc<std::sync::Mutex<Vec<SignalledSwap>>> = Arc::default();
        let captured = signalled.clone();
        let mut clowder = MockClowderClient::new();
        clowder.expect_signal_swap_event().times(2).returning(
            move |_, _, fees, commitment, signatures| {
                let mut captured = captured.lock().unwrap();
                captured.push((fees, commitment, signatures));
                if captured.len() == 1 {
                    Err(Error::ServiceUnavailable)
                } else {
                    Ok(())
                }
            },
        );
        let mut treasury = MockTreasuryService::new();
        treasury
            .expect_store_proofs()
            .times(1)
            .returning(|_| Ok(()));
        let service = service(repository.clone(), clowder, treasury);

        let first = service
            .swap(proofs.clone(), outputs.clone(), commitment, now)
            .await;
        assert!(
            matches!(first, Err(Error::ServiceUnavailable)),
            "a lost Clowder notification must fail the request, got {first:?}"
        );
        assert!(matches!(
            repository.proofs_contains(proofs[0].y().unwrap()).await.unwrap(),
            Some(state) if matches!(state.state, cashu::State::Spent)
        ));
        let (first_fees, _, first_signatures) = signalled.lock().unwrap()[0].clone();
        assert_eq!(
            first_fees
                .iter()
                .fold(cashu::Amount::ZERO, |acc, fee| acc + fee.amount),
            cashu::Amount::from(2u64)
        );

        let retried = service
            .swap(proofs.clone(), outputs.clone(), commitment, now)
            .await
            .expect("retrying the same finalized swap must return its stored signatures");
        assert_eq!(retried, first_signatures);
        for (blind, signature) in outputs.iter().zip(retried.iter()) {
            assert_eq!(
                repository.signature_load(blind).await.unwrap().as_ref(),
                Some(signature)
            );
        }
        assert_eq!(
            signalled.lock().unwrap()[1],
            (first_fees, commitment, retried)
        );

        let other_commitment = signatures_test::random_schnorr_signature();
        let mismatched = service.swap(proofs, outputs, other_commitment, now).await;
        assert!(
            matches!(mismatched, Err(Error::ResourceNotFound(_))),
            "a commitment that did not finalize these inputs must not replay them, got {mismatched:?}"
        );
        assert_eq!(signalled.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn fee_premints_are_derived_from_the_commitment() {
        let repository = Arc::new(inmemory::Repository::default());
        let (_, proofs, outputs, commitment, _) = prepare_swap_with(
            &repository,
            &[cashu::Amount::from(16u64)],
            &[cashu::Amount::from(8u64)],
            SignatureOwner::Unsigned,
        )
        .await;
        let service = service(
            repository,
            MockClowderClient::new(),
            MockTreasuryService::new(),
        );
        let blinded = |premints: Vec<cashu::PreMintSecrets>| {
            premints
                .iter()
                .flat_map(cashu::PreMintSecrets::blinded_messages)
                .collect::<Vec<_>>()
        };

        let first = blinded(
            service
                .generate_fees_premints(&proofs, &outputs, &commitment)
                .await
                .unwrap(),
        );
        let again = blinded(
            service
                .generate_fees_premints(&proofs, &outputs, &commitment)
                .await
                .unwrap(),
        );
        let other = blinded(
            service
                .generate_fees_premints(
                    &proofs,
                    &outputs,
                    &signatures_test::random_schnorr_signature(),
                )
                .await
                .unwrap(),
        );

        assert!(!first.is_empty());
        assert_eq!(first, again);
        assert_ne!(first, other);
    }

    // Expired exchange eCash is burned as issued: no witness, so only the mint signature is checked.
    #[tokio::test]
    async fn burn_accepts_locked_proofs_without_witness() {
        let repository = Arc::new(inmemory::Repository::default());
        let (kinfo, keyset) = core_tests::generate_random_ecash_keyset();
        repository
            .keys_store(keys_utils::to_entry(kinfo, keyset.clone()))
            .await
            .unwrap();
        let locktime = time::OffsetDateTime::now_utc().unix_timestamp() as u64 + 60;
        let wallet = bcr_common::core::generate_random_keypair()
            .public_key()
            .into();
        let mint = bcr_common::core::generate_random_keypair()
            .public_key()
            .into();
        let conditions = bcr_common::core::htlc::exchange_htlc(
            Sha256Hash::hash(b"lock"),
            locktime,
            wallet,
            mint,
        )
        .unwrap();
        let secret = signature::offline_htlc_secret(conditions).unwrap();
        let amount = cashu::Amount::from(8u64);
        let (blinded, r) = cashu::dhke::blind_message(&secret.to_bytes(), None).unwrap();
        let premint = cashu::PreMint {
            secret,
            blinded_message: cashu::BlindedMessage::new(amount, keyset.id.into(), blinded),
            r,
            amount,
        };
        let signed = sign_ecash(&keyset, &premint.blinded_message).unwrap();
        let proof = signature::unblind_ecash_signature(
            &core_keys::to_keyset(&keyset, None),
            premint,
            signed,
        )
        .unwrap();
        assert!(proof.witness.is_none());
        let service = service(
            repository.clone(),
            MockClowderClient::new(),
            MockTreasuryService::new(),
        );

        let ys = service.burn(vec![proof.clone()]).await.unwrap();

        assert_eq!(ys, vec![proof.y().unwrap()]);
        assert!(repository
            .proofs_contains(proof.y().unwrap())
            .await
            .unwrap()
            .is_some());
    }
}
