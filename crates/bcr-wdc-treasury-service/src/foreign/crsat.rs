// ----- standard library imports
use std::{collections::HashMap, str::FromStr, sync::Arc};
// ----- extra library imports
use bcr_wdc_utils::signatures::unblind_signatures;
use bitcoin::hashes::sha256::Hash as Sha256Hash;
use cdk::wallet::MintConnector;
use itertools::Itertools;
// ----- local imports
use crate::{
    error::{Error, Result},
    foreign::proof,
    foreign::{ClowderClient, Repository, KeysClient},
    TStamp,
};

// ----- end imports


#[derive(Clone)]
pub struct Service {
    pub repo: Arc<dyn Repository>,
    pub keys: Arc<dyn KeysClient>,
    pub clowder: Arc<dyn ClowderClient>,
}

impl Service {
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
        let kid = proofs[0].keyset_id;
        self.repo
            .store_htlc(foreign_mint, &htlc_hash, proofs)
            .await?;
        let foreign_keyset = foreign_client.get_mint_keyset(kid).await?;
        let Some(foreign_unix_expiration) = foreign_keyset.final_expiry else {
            return Err(Error::InvalidInput(String::from(
                "Foreign keyset has no expiration",
            )));
        };
        let foreign_date = TStamp::from_timestamp_secs(foreign_unix_expiration as i64)
            .map(|tstamp| tstamp.date_naive());
        let Some(foreign_date) = foreign_date else {
            return Err(Error::InvalidInput(String::from(
                "foreign expiry date parse",
            )));
        };
        let keyset = self.keys.get_keyset_with_expiration(foreign_date).await?;
        let locktime = foreign_locktime - chrono::TimeDelta::minutes(15);
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
        let mut gran_total = cashu::Amount::ZERO;
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
            // TODO: allow different keyset_ids
            assert_eq!(
                1,
                f_proofs.iter().map(|p| p.keyset_id).unique().count(),
                "All foreign proofs must have the same keyset_id"
            );
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
            gran_total += total;
        }
        Ok(gran_total)
    }
}

fn proofs_vec_to_map(
    input: Vec<(cashu::MintUrl, cashu::Proof)>,
) -> HashMap<cashu::MintUrl, Vec<cashu::Proof>> {
    let mut map: HashMap<cashu::MintUrl, Vec<cashu::Proof>> = HashMap::new();
    for (mint, proof) in input {
        map.entry(mint).or_default().push(proof);
    }
    map
}
