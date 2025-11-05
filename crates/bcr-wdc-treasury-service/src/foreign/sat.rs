// ----- standard library imports
use std::{collections::HashSet, ops::Deref, str::FromStr, sync::Arc};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::wire::keys as wire_keys;
use bcr_wdc_utils::signatures::unblind_signatures;
use bitcoin::hashes::sha256::Hash as Sha256Hash;
use cdk::wallet::MintConnector;
// ----- local imports
use crate::{
    error::{Error, Result},
    foreign::{proof, proofs_vec_to_map, ClowderClient, OfflineRepository, OnlineRepository},
};

// ----- end imports

#[async_trait]
pub trait KeysClient: proof::KeysClient {
    async fn get_active_keyset(&self) -> Result<cashu::KeySet>;
}

#[derive(Clone)]
pub struct Service {
    pub online_repo: Arc<dyn OnlineRepository>,
    pub offline_repo: Arc<dyn OfflineRepository>,
    pub keys: Arc<dyn KeysClient>,
    pub clowder: Arc<dyn ClowderClient>,
}

impl Service {
    pub async fn offline_exchange(
        &self,
        inputs: Vec<wire_keys::ProofFingerprint>,
        hashes: Vec<Sha256Hash>,
        wpk: cashu::PublicKey,
    ) -> Result<Vec<cashu::Proof>> {
        let (foreign_mint_url, foreign_mint_pk) = self
            .clowder
            .can_accept_offline_exchange(inputs.clone())
            .await?;
        let keyset = self.keys.get_active_keyset().await?;
        let mut premints = cashu::PreMintSecrets::new(keyset.id);
        for (fp, hash) in inputs.iter().zip(hashes.iter()) {
            let condition = cashu::SpendingConditions::new_htlc_hash(
                &hash.to_string(),
                Some(cashu::Conditions {
                    pubkeys: Some(vec![wpk]),
                    ..Default::default()
                }),
            )?;
            let premint = cashu::PreMintSecrets::with_conditions(
                keyset.id,
                cashu::Amount::from(fp.amount),
                &cashu::amount::SplitTarget::None,
                &condition,
            )?;
            premints.combine(premint);
        }
        let signatures = self.keys.sign(&premints.blinded_messages()).await?;
        let proofs = unblind_signatures(premints, signatures.into_iter(), &keyset)?;
        self.offline_repo
            .store_fps((foreign_mint_pk, foreign_mint_url), inputs, hashes)
            .await?;
        Ok(proofs)
    }

    const HTLC_BUFFER_MINUTES: i64 = 15;
    pub async fn online_exchange(
        &self,
        proofs: Vec<cashu::Proof>,
        path: &[cashu::PublicKey],
    ) -> Result<Vec<cashu::Proof>> {
        if path.len() < 3 {
            return Err(Error::InvalidInput(String::from(
                "Exchange path must be at least [foreign pk, myself pk, wallet pk]",
            )));
        };
        let mut path_it = path.iter().rev();
        let wallet_pk = path_it.next().unwrap();
        let myself = self.clowder.get_myself_pk().await?;
        let myself = cashu::PublicKey::from(myself.inner);
        if &myself != path_it.next().unwrap() {
            return Err(Error::InvalidInput(String::from(
                "Exchange path must end with [myself pk, wallet pk]",
            )));
        };
        let foreign_pk = path_it.next().unwrap();
        let foreign_mint = self.clowder.get_mint_url_from_pk(foreign_pk).await?;
        let foreign_client = cdk::wallet::HttpClient::new(foreign_mint.clone(), None);
        let (htlc_hash, foreign_locktime) = proof::check_htlc_foreign_proofs(
            *foreign_pk,
            &proofs,
            &foreign_client,
            self.clowder.as_ref(),
        )
        .await?;
        let total = proofs
            .iter()
            .fold(cashu::Amount::ZERO, |total, p| total + p.amount);
        self.online_repo
            .store_htlc((*foreign_pk.deref(), foreign_mint), &htlc_hash, proofs)
            .await?;
        let keyset = self.keys.get_active_keyset().await?;
        let locktime = foreign_locktime - chrono::TimeDelta::minutes(Self::HTLC_BUFFER_MINUTES);
        let proofs = proof::generate_htlc_proofs(
            total,
            Some(locktime),
            &htlc_hash,
            *wallet_pk,
            &keyset,
            self.keys.as_ref(),
        )
        .await?;
        Ok(proofs)
    }

    pub async fn try_swap_htlc(&self, preimage: &str) -> Result<cashu::Amount> {
        let online_amount =
            try_online_htlc_swap(preimage, self.online_repo.as_ref(), self.clowder.as_ref())
                .await?;
        if online_amount > cashu::Amount::ZERO {
            return Ok(online_amount);
        }
        let offline_amount =
            try_offline_htlc_swap(preimage, self.offline_repo.as_ref(), self.clowder.as_ref())
                .await?;
        Ok(offline_amount)
    }
}

async fn try_online_htlc_swap(
    preimage: &str,
    repo: &dyn OnlineRepository,
    clowder: &dyn ClowderClient,
) -> Result<cashu::Amount> {
    let mut grand_total = cashu::Amount::ZERO;
    let hash = Sha256Hash::from_str(preimage).map_err(|e| Error::InvalidInput(e.to_string()))?;
    let foreign_proofs = repo.search_htlc(&hash.to_string()).await?;
    let foreign_proofs = proofs_vec_to_map(foreign_proofs);
    for ((mint_pk, mint_url), mut f_proofs) in foreign_proofs {
        let mut f_fingerprints = Vec::with_capacity(f_proofs.len());
        for proof in &mut f_proofs {
            proof.add_preimage(preimage.to_string());
            f_fingerprints.push(proof.y()?);
        }
        f_proofs = clowder.sign_p2pk_proofs(&f_proofs).await?;
        let total = f_proofs
            .iter()
            .fold(cashu::Amount::ZERO, |total, p| total + p.amount);
        let premints = cashu::PreMintSecrets::random(
            f_proofs[0].keyset_id,
            total,
            &cashu::amount::SplitTarget::None,
        )
        .map_err(|e| Error::Internal(e.to_string()))?;
        if f_proofs
            .iter()
            .map(|p| p.keyset_id)
            .collect::<HashSet<_>>()
            .len()
            != 1
        {
            return Err(Error::InvalidInput(
                "All foreign proofs must have the same keyset_id".to_string(),
            ));
        }
        let foreign_client = cdk::wallet::HttpClient::new(mint_url.clone(), None);
        let keys = foreign_client
            .get_mint_keyset(f_proofs[0].keyset_id)
            .await?;
        let request = cashu::SwapRequest::new(f_proofs, premints.blinded_messages());
        let swap_response = foreign_client.post_swap(request).await?;
        let proofs = unblind_signatures(premints, swap_response.signatures.into_iter(), &keys)
            .map_err(|e| Error::Internal(e.to_string()))?;
        repo.store((mint_pk, mint_url), proofs).await?;
        repo.remove_htlcs(&f_fingerprints).await?;
        grand_total += total;
    }
    Ok(grand_total)
}

async fn try_offline_htlc_swap(
    preimage: &str,
    repo: &dyn OfflineRepository,
    clowder: &dyn ClowderClient,
) -> Result<cashu::Amount> {
    let hash = Sha256Hash::from_str(preimage).map_err(|e| Error::InvalidInput(e.to_string()))?;
    let Some(((mint_pk, mint_url), fp)) = repo.search_fp(&hash).await? else {
        return Ok(cashu::Amount::ZERO);
    };
    let secret = cashu::secret::Secret::from_str(preimage)?;
    let amount = cashu::Amount::from(fp.amount);
    let proof = cashu::Proof {
        amount,
        keyset_id: fp.keyset_id,
        c: fp.c,
        dleq: fp.dleq,
        witness: fp.witness,
        secret,
    };
    if proof.y()? != fp.y {
        return Err(Error::InvalidInput(String::from(
            "preimage does not match fingerprint",
        )));
    }
    let keys = clowder.get_keyset(&mint_pk, &proof.keyset_id).await?;
    let key = keys
        .keys
        .get(&proof.amount)
        .ok_or(Error::Internal(String::from("key amount not found")))?;
    proof.verify_dleq(*key)?;
    repo.remove_fps(&[fp.y]).await?;
    repo.store_proofs((mint_pk, mint_url), vec![proof]).await?;
    Ok(amount)
}
