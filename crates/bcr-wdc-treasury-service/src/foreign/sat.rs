// ----- standard library imports
use cdk::wallet::MintConnector;
use std::{collections::HashSet, str::FromStr, sync::Arc};
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_utils::signatures::unblind_signatures;
use bitcoin::hashes::sha256::Hash as Sha256Hash;
// ----- local imports
use crate::{
    error::{Error, Result},
    foreign::proof,
    foreign::{proofs_vec_to_map, ClowderClient, Repository},
};

// ----- end imports

#[async_trait]
pub trait KeysClient: proof::KeysClient {
    async fn get_active_keyset(&self) -> Result<cashu::KeySet>;
}

#[derive(Clone)]
pub struct Service {
    pub repo: Arc<dyn Repository>,
    pub keys: Arc<dyn KeysClient>,
    pub clowder: Arc<dyn ClowderClient>,
}

impl Service {
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
        self.repo
            .store_htlc(foreign_mint, &htlc_hash, proofs)
            .await?;
        let keyset = self.keys.get_active_keyset().await?;
        let locktime = foreign_locktime - chrono::TimeDelta::minutes(Self::HTLC_BUFFER_MINUTES);
        let proofs = proof::generate_htlc_proofs(
            total,
            locktime,
            &htlc_hash,
            *wallet_pk,
            &keyset,
            self.keys.as_ref(),
        )
        .await?;
        Ok(proofs)
    }

    pub async fn try_swap_htlc(&self, preimage: &str) -> Result<cashu::Amount> {
        let mut grand_total = cashu::Amount::ZERO;
        let hash =
            Sha256Hash::from_str(preimage).map_err(|e| Error::InvalidInput(e.to_string()))?;
        let foreign_proofs = self.repo.search_htlc(&hash.to_string()).await?;
        let foreign_proofs = proofs_vec_to_map(foreign_proofs);
        for (mint, mut f_proofs) in foreign_proofs {
            let mut f_fingerprints = Vec::with_capacity(f_proofs.len());
            for proof in &mut f_proofs {
                proof.add_preimage(preimage.to_string());
                f_fingerprints.push(proof.y()?);
            }
            f_proofs = self.clowder.sign_p2pk_proofs(&f_proofs).await?;
            let total = f_proofs
                .iter()
                .fold(cashu::Amount::ZERO, |total, p| total + p.amount);
            let premints = cashu::PreMintSecrets::random(
                f_proofs[0].keyset_id,
                total,
                &cashu::amount::SplitTarget::None,
            )
            .map_err(|e| Error::Internal(e.to_string()))?;
            if f_proofs.iter().map(|p| p.keyset_id).collect::<HashSet<_>>().len() != 1 {
                return Err(Error::InvalidInput(
                    "All foreign proofs must have the same keyset_id".to_string(),
                ));
            }
            let foreign_client = cdk::wallet::HttpClient::new(mint.clone(), None);
            let keys = foreign_client
                .get_mint_keyset(f_proofs[0].keyset_id)
                .await?;
            let request = cashu::SwapRequest::new(f_proofs, premints.blinded_messages());
            let swap_response = foreign_client.post_swap(request).await?;
            let proofs = unblind_signatures(premints, swap_response.signatures.into_iter(), &keys)
                .map_err(|e| Error::Internal(e.to_string()))?;
            self.repo.store(mint, proofs).await?;
            self.repo.remove_htlcs(&f_fingerprints).await?;
            grand_total += total;
        }
        Ok(grand_total)
    }
}
