// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bcr_common::cashu;
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
            debug!("Checking offline status for mint {}", mint_id);
            let mint_url = try_or_warn!(self.clowder.get_mint_url_from_pk(&mint_id));
            let is_offline = try_or_warn!(self.clowder.is_offline(mint_id));
            if is_offline {
                debug!("{} still offline", mint_url);
                continue;
            }
            let client = try_or_warn!(self.mint_factory.make_client(mint_url.clone(), mint_id));
            let proofs = try_or_warn!(self.offline.load_proofs(mint_id));
            let mapped_proofs = bcr_common::core::signature::proofs_to_map(proofs);
            for (kid, proofs) in mapped_proofs {
                let total = proofs
                    .iter()
                    .fold(cashu::Amount::ZERO, |acc, p| acc + p.amount);
                let ys: Vec<cashu::PublicKey> = proofs
                    .iter()
                    .map(|p| p.y().expect("Proof::y(): impossible!!"))
                    .collect();
                let keyset = try_or_warn!(client.get_keyset(kid));
                debug!(
                    "Settling proofs totaling {total} at {} with keysetID {kid}",
                    mint_url
                );
                let Ok(premints) = cashu::PreMintSecrets::random(
                    kid,
                    total,
                    &cashu::amount::SplitTarget::None,
                    &bcr_wdc_utils::keys::to_fee_and_amounts(&keyset),
                ) else {
                    warn!("PreMintSecrets::random failed {}, retry later", mint_url);
                    continue;
                };
                let signatures =
                    try_or_warn!(client.swap(proofs, premints.blinded_messages(), now));
                let (rs, secrets) = premints
                    .secrets
                    .into_iter()
                    .map(|s| (s.r, s.secret))
                    .unzip();
                let news = cashu::dhke::construct_proofs(signatures, rs, secrets, &keyset.keys)?;
                if self.online.store(mint_id, news).await.is_err() {
                    error!("store new proofs failed {}, {total} lost", mint_url);
                };
                if self.offline.remove_proofs(&ys).await.is_err() {
                    warn!("remove_proofs failed {}", mint_url);
                }
                info!("successfully settled {total} at {}", mint_url);
            }
        }
        Ok(None)
    }
}
