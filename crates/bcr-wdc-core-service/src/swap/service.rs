// ----- standard library imports
use std::collections::HashSet;
// ----- extra library imports
use bcr_common::cashu;
use bcr_wdc_utils::signatures as signatures_utils;
use futures::future::JoinAll;
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence::ProofRepository,
    swap::{ClowderClient, SigningService},
};

// ----- end imports

pub struct Service {
    pub proofs: Box<dyn ProofRepository>,
    pub clowder: Box<dyn ClowderClient>,
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
