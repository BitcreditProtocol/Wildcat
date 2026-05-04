// ----- standard library imports
use std::sync::{Arc, Mutex, Weak};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::cashu;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
// ----- local imports
use crate::{
    error::{Error, Result},
    foreign::{
        ClowderClient, MintClientFactory, OfflineRepository, OfflineSettleHandler, OnlineRepository,
    },
};

// ----- end imports

pub struct Handler {
    online: Weak<dyn OnlineRepository>,
    offline: Weak<dyn OfflineRepository>,
    clowder: Weak<dyn ClowderClient>,
    mint_factory: Weak<dyn MintClientFactory>,
    cancel: CancellationToken,
    interval: tokio::time::Duration,
    monitored: Mutex<Vec<secp256k1::PublicKey>>,
    handles: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl Handler {
    pub fn new(
        online: &Arc<dyn OnlineRepository>,
        offline: &Arc<dyn OfflineRepository>,
        clowder: &Arc<dyn ClowderClient>,
        mint_factory: &Arc<dyn MintClientFactory>,
        interval: std::time::Duration,
    ) -> Self {
        Self {
            online: Arc::downgrade(online),
            offline: Arc::downgrade(offline),
            clowder: Arc::downgrade(clowder),
            mint_factory: Arc::downgrade(mint_factory),
            cancel: CancellationToken::new(),
            interval,
            monitored: Mutex::new(Vec::new()),
            handles: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl OfflineSettleHandler for Handler {
    fn monitor(&self, mint: (secp256k1::PublicKey, reqwest::Url)) -> Result<()> {
        {
            let mut monitored = self.monitored.lock().unwrap();
            if monitored.contains(&mint.0) {
                return Ok(());
            }
            monitored.push(mint.0);
        }
        let handle = tokio::spawn(monitor(
            mint.clone(),
            Weak::clone(&self.clowder),
            Weak::clone(&self.online),
            Weak::clone(&self.offline),
            Weak::clone(&self.mint_factory),
            self.interval,
            self.cancel.clone(),
        ));
        {
            let mut handles = self.handles.lock().unwrap();
            handles.push(handle);
        }
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.cancel.cancel();
        loop {
            let handle = self.handles.lock().unwrap().pop();
            let Some(handle) = handle else { return Ok(()) };
            handle.await.map_err(|e| Error::Internal(e.to_string()))?;
        }
    }
}

async fn monitor(
    mint: (secp256k1::PublicKey, reqwest::Url),
    clowder: Weak<dyn ClowderClient>,
    online: Weak<dyn OnlineRepository>,
    offline: Weak<dyn OfflineRepository>,
    factory: Weak<dyn MintClientFactory>,
    pause: Duration,
    cancel: CancellationToken,
) {
    debug!("Starting offline monitor for {}", mint.1);
    loop {
        tokio::select! {
                _ = cancel.cancelled() => {
                    debug!("Monitor cancelled, abandoning {}", mint.1);
                    break;
                }
                _ = tokio::time::sleep(pause) => {
            }
        }
        debug!("Checking offline status for {}", mint.1);
        let Some(clowder) = clowder.upgrade() else {
            warn!("ClowderClient upgrade failed, abandoning {}", mint.1);
            break;
        };
        let Ok(is_offline) = clowder.is_offline(mint.0).await else {
            warn!("is_offline failed, retrying later {}", mint.1);
            continue;
        };
        if is_offline {
            debug!("{} still offline", mint.1);
            continue;
        }
        // process settlement
        let Some(offline_repo) = offline.upgrade() else {
            warn!("OfflineRepository upgrade failed, abandoning {}", mint.1);
            break;
        };
        let Some(online_repo) = online.upgrade() else {
            warn!("OnlineRepository upgrade failed, abandoning {}", mint.1);
            break;
        };
        let Some(factory) = factory.upgrade() else {
            warn!("MintClientFactory upgrade failed, abandoning {}", mint.1);
            break;
        };
        let Ok(client) = factory.make_client(mint.1.clone(), mint.0).await else {
            warn!("make_client failed {}, retry later", mint.1);
            continue;
        };
        let Ok(proofs) = offline_repo.load_proofs(mint.0).await else {
            warn!("load_proofs failed {}, retry later", mint.1);
            continue;
        };
        let proofs = bcr_common::core::signature::proofs_to_map(proofs);
        for (kid, proofs) in proofs {
            let total = proofs
                .iter()
                .fold(cashu::Amount::ZERO, |acc, p| acc + p.amount);
            let mut ys = Vec::with_capacity(proofs.len());
            for proof in &proofs {
                let Ok(y) = proof.y() else {
                    warn!("Proof::y() failed {}, retry later", mint.1);
                    continue;
                };
                ys.push(y);
            }
            let Ok(keyset) = client.get_keyset(kid).await else {
                warn!("get_keyset failed {}, retry later", mint.1);
                continue;
            };
            debug!(
                "Settling proofs totaling {total} at {} with keysetID {kid}",
                mint.1
            );
            let Ok(premints) =
                cashu::PreMintSecrets::random(kid, total, &cashu::amount::SplitTarget::None)
            else {
                warn!("PreMintSecrets::random failed {}, retry later", mint.1);
                continue;
            };
            let now = chrono::Utc::now();
            let Ok(signatures) = client.swap(proofs, premints.blinded_messages(), now).await else {
                warn!("swap failed {}, lost {}", mint.1, total);
                continue;
            };
            let mut news = Vec::with_capacity(signatures.len());
            for (signature, premint) in signatures.into_iter().zip(premints.iter()) {
                let amount = signature.amount;
                let Ok(proof) = bcr_common::core::signature::unblind_ecash_signature(
                    &keyset,
                    premint.clone(),
                    signature,
                ) else {
                    error!("unblind_ecash_signature failed {}, lost {}", mint.1, amount);
                    continue;
                };
                news.push(proof);
            }
            if online_repo.store(mint.0, news).await.is_err() {
                error!("store new proofs failed {}, {total} lost", mint.1);
            };
            if offline_repo.remove_proofs(&ys).await.is_err() {
                warn!("remove_proofs failed {}", mint.1);
            }
            info!("successfully settled {total} at {}", mint.1);
        }
        break;
    }
}
