// ----- standard library imports
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bcr_common::cashu::{self, ProofsMethods};
use bcr_wdc_utils::routine::{Routine, TStamp};
use tracing::{debug, error, info, warn};
// ----- local imports
use crate::foreign::{ClowderClient, MintClientFactory, OfflineRepository, OnlineRepository};

// ----- end imports

pub struct Handler {
    pub online: Arc<dyn OnlineRepository>,
    pub offline: Arc<dyn OfflineRepository>,
    pub clowder: Arc<dyn ClowderClient>,
    pub mint_factory: Arc<dyn MintClientFactory>,
}

macro_rules! try_or_warn {
    ($expr:expr) => {
        match $expr {
            Ok(result) => result,
            Err(e) => {
                warn!("{} failed: {}, retry later", stringify!($expr), e);
                continue;
            }
        }
    };
}
macro_rules! async_try_or_warn {
    ($expr:expr) => {
        match $expr.await {
            Ok(result) => result,
            Err(e) => {
                warn!("{} failed: {}, retry later", stringify!($expr), e);
                continue;
            }
        }
    };
}

#[async_trait]
impl Routine for Handler {
    async fn run_task(&self, now: TStamp) -> AnyResult<Option<std::time::Duration>> {
        let mints = self.offline.list_foreign_pks().await?;
        for mint_id in mints {
            debug!("Checking offline status for mint {mint_id}");
            let mint_url = async_try_or_warn!(self.clowder.get_mint_url_from_pk(&mint_id));
            let is_offline = async_try_or_warn!(self.clowder.is_offline(mint_id));
            if is_offline {
                debug!("{} still offline", mint_url);
                continue;
            }
            let client =
                async_try_or_warn!(self.mint_factory.make_client(mint_url.clone(), mint_id));
            let foreign_proofs = async_try_or_warn!(self.offline.load_proofs(mint_id));
            let foreign_total = try_or_warn!(foreign_proofs.total_amount());
            let foreign_ys: Vec<cashu::PublicKey> = foreign_proofs
                .iter()
                .map(|p| p.y().expect("Proof::y(): impossible!!"))
                .collect();
            let foreign_kids: HashSet<cashu::Id> =
                foreign_proofs.iter().map(|p| p.keyset_id).collect();
            let mut foreign_kinfos = HashMap::new();
            for foreign_kid in foreign_kids {
                let kinfo =
                    async_try_or_warn!(self.clowder.get_keyset_info(&mint_id, &foreign_kid));
                foreign_kinfos.insert(foreign_kid, kinfo);
            }
            let swap_plan = try_or_warn!(bcr_common::core::swap::wallet::prepare_swap(
                &foreign_proofs,
                &foreign_kinfos,
            ));
            let mut foreign_keysets_map: HashMap<cashu::Id, cashu::KeySet> = HashMap::new();
            let mut premint_secrets: Vec<cashu::PreMintSecrets> =
                Vec::with_capacity(swap_plan.len());
            for (kid, amount) in swap_plan {
                let keyset = async_try_or_warn!(self.clowder.get_keyset(&mint_id, &kid));
                let Ok(premint) = cashu::PreMintSecrets::random(
                    kid,
                    amount,
                    &cashu::amount::SplitTarget::None,
                    &bcr_wdc_utils::keys::to_fee_and_amounts(&keyset),
                ) else {
                    warn!("PreMintSecrets::random failed {mint_url}, retry later");
                    continue;
                };
                premint_secrets.push(premint);
                foreign_keysets_map.insert(kid, keyset);
            }
            let blinds: Vec<cashu::BlindedMessage> = premint_secrets
                .iter()
                .flat_map(|p| p.blinded_messages())
                .collect();
            let premints: Vec<cashu::PreMint> = premint_secrets
                .into_iter()
                .flat_map(|p| p.secrets)
                .collect();
            let signatures = async_try_or_warn!(client.swap(foreign_proofs, blinds, now));
            let mut new_proofs = Vec::with_capacity(signatures.len());
            for (signature, premint) in signatures.into_iter().zip(premints) {
                let keyset = foreign_keysets_map
                    .get(&signature.keyset_id)
                    .expect("keyset_id must be here");
                let new_p = try_or_warn!(bcr_common::core::signature::unblind_ecash_signature(
                    keyset, premint, signature
                ));
                new_proofs.push(new_p);
            }
            loop {
                match self.online.store(mint_id, new_proofs.clone()).await {
                    Ok(_) => break,
                    Err(e) => {
                        error!("Failed to store proofs for mint {mint_url}: {e}, retrying...");
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    }
                }
            }
            if self.offline.remove_proofs(&foreign_ys).await.is_err() {
                warn!("remove_proofs failed {mint_url}");
            }
            info!("Settled {foreign_total} from mint {mint_url}");
        }
        Ok(None)
    }
}
