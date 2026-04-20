// ----- standard library imports
use std::collections::HashSet;
// ----- extra library imports
use bcr_common::{cashu, core::signature, wire::swap as wire_swap};
use bcr_wdc_utils::signatures as signatures_utils;
use futures::future::JoinAll;
use secp256k1::schnorr;
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence::{CommitmentRepository, ProofRepository},
    swap::{ClowderClient, SigningService},
    TStamp,
};

// ----- end imports

pub struct Service {
    pub proofs: Box<dyn ProofRepository>,
    pub commitments: Box<dyn CommitmentRepository>,
    pub clowder: Box<dyn ClowderClient>,
    pub max_expiry: chrono::Duration,
}

impl Service {
    async fn are_keysets_active(
        &self,
        sign_service: &dyn SigningService,
        kids: impl Iterator<Item = &cashu::Id>,
    ) -> Result<Vec<(cashu::Id, bool)>> {
        let joined: JoinAll<_> = kids.map(|kid| sign_service.info(kid)).collect();
        let responses: Vec<_> = joined.await.into_iter().collect::<Result<_>>()?;
        let statuses = responses
            .into_iter()
            .map(|info| (info.id, info.active))
            .collect();
        Ok(statuses)
    }

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
        sign_service: &dyn SigningService,
        request: wire_swap::SwapCommitmentRequest,
        now: TStamp,
    ) -> Result<(String, schnorr::Signature)> {
        // check wallet signature
        signature::schnorr_verify_b64(
            &request.content,
            &request.wallet_signature,
            &request.wallet_key.x_only_public_key(),
        )?;
        let content: wire_swap::SwapCommitmentRequestBody =
            signature::deserialize_borsh_msg(&request.content)?;
        // check expiry
        let expiry = chrono::DateTime::from_timestamp(content.expiry as i64, 0)
            .ok_or_else(|| Error::InvalidInput("invalid expiry timestamp".into()))?;
        if expiry <= now {
            return Err(Error::InvalidInput("commitment already expired".into()));
        }
        let max_allowed = now + self.max_expiry;
        let expiry = expiry.min(max_allowed);
        // basic checks
        let core_fps = content
            .inputs
            .iter()
            .map(|fp| signature::ProofFingerprint::from(fp.clone()))
            .collect::<Vec<_>>();
        signatures_utils::basic_fingerprints_checks(&core_fps)?;
        signatures_utils::basic_blinds_checks(&content.outputs)?;
        // check amounts
        // TODO: fees are not considered
        let input_amount: u64 = content.inputs.iter().map(|fp| fp.amount).sum();
        let output_amount: u64 = content.outputs.iter().map(|b| u64::from(b.amount)).sum();
        if input_amount != output_amount {
            return Err(Error::InvalidInput(format!(
                "amount mismatch: inputs={input_amount}, outputs={output_amount}"
            )));
        }
        // check inputs are unspent
        let ys: Vec<cashu::PublicKey> = content.inputs.iter().map(|fp| fp.y).collect();
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
        let bs: Vec<cashu::PublicKey> = content.outputs.iter().map(|b| b.blinded_secret).collect();
        let contained = self.commitments.contains_outputs(&bs).await?;
        if contained {
            return Err(Error::InvalidInput(String::from(
                "blinded messages committed",
            )));
        }
        let wallet_pk = request.wallet_key;
        let wallet_sig = request.wallet_signature;
        // broadcast request to clowder
        let (content, commitment) = self.clowder.commit_to_swap(request).await?;
        // store commitment
        let store_res = self
            .commitments
            .store(ys, bs, expiry, wallet_pk, wallet_sig, commitment.clone())
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
        sign_service: &dyn SigningService,
        inputs: &[cashu::Proof],
        outputs: &[cashu::BlindedMessage],
    ) -> Result<Vec<cashu::BlindSignature>> {
        // cheap verifications
        signatures_utils::basic_proofs_checks(inputs)?;
        signatures_utils::basic_blinds_checks(outputs)?;
        // 3. inputs and outputs grouped by keyset ID have equal amounts
        let unique_ids: HashSet<_> = inputs.iter().map(|p| p.keyset_id).collect();
        for id in &unique_ids {
            let total_input = inputs
                .iter()
                .filter(|p| p.keyset_id == *id)
                .fold(cashu::Amount::ZERO, |total, proof| total + proof.amount);
            let total_output = outputs
                .iter()
                .filter(|p| p.keyset_id == *id)
                .fold(cashu::Amount::ZERO, |total, proof| total + proof.amount);
            // TODO: fees are not considered
            if total_input != total_output {
                return Err(Error::InvalidInput(format!(
                    "input/output mismatch {total_input}/{total_output}",
                )));
            }
        }
        // expensive verifications
        // 1. verify keysets are active
        let statuses = self
            .are_keysets_active(sign_service, unique_ids.iter())
            .await?;
        for (id, status) in statuses.iter() {
            if !status {
                return Err(Error::InactiveKeyset(*id));
            }
        }
        // 2. verify proofs signatures
        sign_service.verify_proofs(inputs).await?;
        // generate signatures
        let signatures = sign_service.sign_blinds(outputs).await?;
        self.proofs.insert(inputs).await?;
        Ok(signatures)
    }

    pub async fn burn(
        &self,
        sign_service: &dyn SigningService,
        proofs: &[cashu::Proof],
    ) -> Result<Vec<cashu::PublicKey>> {
        // cheap verifications
        signatures_utils::basic_proofs_checks(proofs)?;
        // expensive verifications
        let unique_ids: HashSet<_> = proofs.iter().map(|p| p.keyset_id).collect();
        // 1. verify keysets are inactive
        let statuses = self
            .are_keysets_active(sign_service, unique_ids.iter())
            .await?;
        for (id, status) in statuses.iter() {
            if *status {
                return Err(Error::ActiveKeyset(*id));
            }
        }
        // 2. verify proofs signatures
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
