// ----- standard library imports
use std::sync::{Arc, Mutex};
// ----- extra library imports
use async_trait::async_trait;
use bdk_electrum::BdkElectrumClient;
use bdk_wallet::{
    bitcoin::{
        self as btc,
        bip32::Xpriv,
        hashes::{sha256, Hash},
        Network,
    },
    descriptor::template::Bip84,
    keys::{bip39::Mnemonic, DerivableKey, ExtendedKey},
    miniscript::{descriptor::KeyMap, Descriptor, DescriptorPublicKey},
    rusqlite::OpenFlags,
    KeychainKind,
};
use futures::future::JoinAll;
use rand::Rng;
use serde_with::serde_as;
use tokio_util::sync::CancellationToken;
// ----- local imports
use crate::error::{Error, Result};
use crate::service::OnChainWallet;

// ----- end imports

pub type BdkWallet = bdk_wallet::PersistedWallet<bdk_wallet::rusqlite::Connection>;
pub type PersistedBdkWallet = (BdkWallet, bdk_wallet::rusqlite::Connection);
pub type SingleSecretKeyDescriptor = (Descriptor<DescriptorPublicKey>, KeyMap);

#[serde_as]
#[derive(Debug, Clone, serde::Deserialize)]
pub struct WalletConfig {
    mnemonic: Mnemonic,
    network: Network,
    store_path: std::path::PathBuf,
    stop_gap: usize,
    #[serde_as(as = "serde_with::DurationSeconds<i64>")]
    update_interval: chrono::Duration,
}

#[async_trait]
pub trait PrivateKeysRepository {
    async fn get_private_keys(&self) -> Result<Vec<SingleSecretKeyDescriptor>>;
    async fn add_key(&self, key: SingleSecretKeyDescriptor) -> Result<()>;
}

type SyncingWallet = (Arc<Mutex<PersistedBdkWallet>>, CancellationToken);

#[derive(Debug)]
pub struct Wallet<KeyRepo, ElectrumApi> {
    main: SyncingWallet,
    // each wallet has its own updating loop task
    // the vector is mutating as keys are added and removed
    onetimes: Arc<Mutex<Vec<SyncingWallet>>>,
    store_path: std::path::PathBuf,
    update_interval: core::time::Duration,
    repo: KeyRepo,
    electrum_client: Arc<BdkElectrumClient<ElectrumApi>>,
    network: Network,
    stop_gap: usize,
}

impl<KeyRepo, ElectrumApi> Wallet<KeyRepo, ElectrumApi> {
    pub fn network(&self) -> Network {
        self.network
    }
}

impl<KeyRepo, ElectrumApi> std::clone::Clone for Wallet<KeyRepo, ElectrumApi>
where
    KeyRepo: Clone,
{
    fn clone(&self) -> Self {
        Self {
            main: self.main.clone(),
            onetimes: self.onetimes.clone(),
            store_path: self.store_path.clone(),
            update_interval: self.update_interval,
            repo: self.repo.clone(),
            electrum_client: self.electrum_client.clone(),
            network: self.network,
            stop_gap: self.stop_gap,
        }
    }
}

impl<KeyRepo, ElectrumApi> Wallet<KeyRepo, ElectrumApi>
where
    KeyRepo: PrivateKeysRepository,
    ElectrumApi: electrum_client::ElectrumApi + Send + Sync + 'static,
{
    const MAIN_STORE_FNAME: &'static str = "main.sqlite";

    pub async fn new(cfg: WalletConfig, repo: KeyRepo, api: ElectrumApi) -> Result<Self> {
        if !cfg.store_path.is_dir() {
            return Err(Error::OnChainStore(cfg.store_path));
        }

        let update_interval = cfg.update_interval.to_std()?;
        let electrum_client = Arc::new(BdkElectrumClient::new(api));

        let exkey: ExtendedKey = cfg.mnemonic.into_extended_key()?;
        let xpriv = exkey.into_xprv(cfg.network).ok_or(Error::MnemonicToXpriv)?;
        let main_store = cfg.store_path.join(Self::MAIN_STORE_FNAME);
        let (age, main) = initialize_main_wallet(&main_store, xpriv, cfg.network)?;
        let main = Arc::new(Mutex::new(main));

        let interval = random_update_interval(update_interval);
        let token = CancellationToken::new();
        let main = (main, token);
        tokio::spawn(wallet_update_loop(
            main.clone(),
            age,
            electrum_client.clone(),
            cfg.stop_gap,
            interval,
        ));

        let keys = repo.get_private_keys().await?;
        let joined: JoinAll<_> = keys
            .into_iter()
            .map(|key| async {
                let (age, wlt) = initialize_single_wallet(&cfg.store_path, key, cfg.network)?;
                let wlt = Arc::new(Mutex::new(wlt));
                let interval = random_update_interval(update_interval);
                let token = CancellationToken::new();
                let active_wlt = (wlt, token);
                tokio::spawn(wallet_update_loop(
                    active_wlt.clone(),
                    age,
                    electrum_client.clone(),
                    cfg.stop_gap,
                    interval,
                ));
                Ok(active_wlt)
            })
            .collect();
        let onetimes: Vec<SyncingWallet> = joined.await.into_iter().collect::<Result<_>>()?;

        Ok(Self {
            main,
            onetimes: Arc::new(Mutex::new(onetimes)),
            repo,
            update_interval,
            store_path: cfg.store_path,
            electrum_client,
            network: cfg.network,
            stop_gap: cfg.stop_gap,
        })
    }
}

#[async_trait]
impl<KeyRepo, ElectrumApi> OnChainWallet for Wallet<KeyRepo, ElectrumApi>
where
    KeyRepo: PrivateKeysRepository + Sync,
    ElectrumApi: electrum_client::ElectrumApi + Sync + Send + 'static,
{
    fn generate_new_recipient(&self) -> Result<btc::Address> {
        let mut locked = self.main.0.lock().unwrap();
        let (wlt, db) = &mut *locked;
        let address_info = wlt.reveal_next_address(KeychainKind::External);
        wlt.persist(db)?;
        Ok(address_info.address)
    }

    fn balance(&self) -> Result<bdk_wallet::Balance> {
        let mut balance = {
            let locked = self.main.0.lock().unwrap();
            let (wlt, _) = &*locked;
            wlt.balance()
        };
        let locked_vec = self.onetimes.lock().unwrap();
        for (wlt, _) in locked_vec.iter() {
            let locked = wlt.lock().unwrap();
            let (wlt, _) = &*locked;
            balance = balance + wlt.balance();
        }
        Ok(balance)
    }

    async fn add_descriptor(&self, descriptor: &str) -> Result<btc::Address> {
        let desc: SingleSecretKeyDescriptor = {
            let locked = self.main.0.lock().unwrap();
            let (wlt, _) = &*locked;
            let secp_ctx = wlt.secp_ctx();
            Descriptor::parse_descriptor(secp_ctx, descriptor)?
        };
        self.repo.add_key(desc.clone()).await?;
        let (age, mut wlt) = initialize_single_wallet(&self.store_path, desc, self.network)?;
        let addr_info = wlt.0.reveal_next_address(KeychainKind::External);
        wlt.0.persist(&mut wlt.1)?;
        let wlt = Arc::new(Mutex::new(wlt));
        let interval = random_update_interval(self.update_interval);
        let token = CancellationToken::new();
        let active_wlt = (wlt, token);
        tokio::spawn(wallet_update_loop(
            active_wlt.clone(),
            age,
            self.electrum_client.clone(),
            self.stop_gap,
            interval,
        ));
        let mut locked = self.onetimes.lock().unwrap();
        locked.push(active_wlt);
        Ok(addr_info.address)
    }

    fn get_address_balance(&self, addr: &btc::Address) -> Result<btc::Amount> {
        let script = addr.script_pubkey();
        {
            let locked = self.main.0.lock().unwrap();
            let (wlt, _) = &*locked;
            if wlt.is_mine(script.clone()) {
                let total: btc::Amount = wlt
                    .list_unspent()
                    .filter(|output| !output.is_spent)
                    .filter(|output| output.txout.script_pubkey == script)
                    .fold(btc::Amount::ZERO, |sum, output| sum + output.txout.value);
                return Ok(total);
            }
        }
        {
            let locked = self.onetimes.lock().unwrap();
            for (active_wlt, _) in locked.iter() {
                let wlt_locked = active_wlt.lock().unwrap();
                let (wlt, _) = &*wlt_locked;
                if wlt.is_mine(script.clone()) {
                    let total: btc::Amount = wlt
                        .list_unspent()
                        .filter(|output| !output.is_spent)
                        .filter(|output| output.txout.script_pubkey == script)
                        .fold(btc::Amount::ZERO, |sum, output| sum + output.txout.value);
                    return Ok(total);
                }
            }
        }
        Err(Error::UnknownAddress(addr.clone()))
    }
}

enum WalletAge {
    New,
    Old,
}

fn initialize_main_wallet(
    store_file: &std::path::Path,
    xpriv: Xpriv,
    network: Network,
) -> Result<(WalletAge, PersistedBdkWallet)> {
    let internal = Bip84(xpriv, KeychainKind::Internal);
    let external = Bip84(xpriv, KeychainKind::External);

    let mut conn = bdk_wallet::rusqlite::Connection::open_with_flags(
        store_file,
        OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_READ_WRITE,
    )?;
    let wallet_opt = bdk_wallet::LoadParams::new()
        .descriptor(KeychainKind::Internal, Some(internal.clone()))
        .descriptor(KeychainKind::External, Some(external.clone()))
        .extract_keys()
        .check_network(network)
        .load_wallet(&mut conn)?;
    match wallet_opt {
        Some(wallet) => Ok((WalletAge::Old, (wallet, conn))),
        None => {
            let wallet = bdk_wallet::CreateParams::new(external, internal)
                .network(network)
                .create_wallet(&mut conn)?;
            Ok((WalletAge::New, (wallet, conn)))
        }
    }
}

fn initialize_single_wallet(
    store_path: &std::path::Path,
    (desc, kmap): SingleSecretKeyDescriptor,
    network: Network,
) -> Result<(WalletAge, PersistedBdkWallet)> {
    let fname = sha256::Hash::hash(desc.to_string().as_bytes()).to_string() + ".sqlite";
    let store = store_path.join(fname);
    let mut conn = bdk_wallet::rusqlite::Connection::open_with_flags(
        store,
        OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_READ_WRITE,
    )?;
    let wallet_opt = bdk_wallet::LoadParams::new()
        .descriptor(KeychainKind::External, Some((desc.clone(), kmap.clone())))
        .extract_keys()
        .check_network(network)
        .load_wallet(&mut conn)?;
    match wallet_opt {
        Some(wallet) => Ok((WalletAge::Old, (wallet, conn))),
        None => {
            let wallet = bdk_wallet::CreateParams::new_single((desc, kmap))
                .network(network)
                .create_wallet(&mut conn)?;
            Ok((WalletAge::New, (wallet, conn)))
        }
    }
}

async fn wallet_update_loop<ElectrumApi>(
    wlt: SyncingWallet,
    age: WalletAge,
    electrum_client: Arc<BdkElectrumClient<ElectrumApi>>,
    stop_gap: usize,
    pause: core::time::Duration,
) where
    ElectrumApi: electrum_client::ElectrumApi,
{
    let (wlt, token) = wlt;
    if matches!(age, WalletAge::New) {
        let request = {
            let mut locked = wlt.lock().unwrap();
            let (wallet, _) = &mut *locked;
            wallet.start_full_scan()
        };
        let result = electrum_client.full_scan(request, stop_gap, 1, false);
        if result.is_err() {
            log::error!("full scan error: {}", result.unwrap_err());
            return;
        }
    }
    loop {
        tokio::select! {
            _ = token.cancelled() => {
                log::info!("wallet update loop stopping");
                break;
            }
            _ = tokio::time::sleep(pause) => {
                log::debug!("wallet update loop waking up");
            }
        }

        let request = {
            let mut locked = wlt.lock().unwrap();
            let (wallet, _) = &mut *locked;
            wallet.start_sync_with_revealed_spks()
        };
        let result = electrum_client.sync(request, 1, false);
        match result {
            Err(e) => {
                log::error!("sync error: {}", e);
                continue;
            }
            Ok(update) => {
                let mut locked = wlt.lock().unwrap();
                let (wallet, db) = &mut *locked;
                wallet.apply_update(update).unwrap();
                wallet.persist(db).unwrap();
            }
        }
    }
}

// random interval to spread out the load on the electrum server
// given the average interval, we spread it by +/- 25%
fn random_update_interval(avg: core::time::Duration) -> core::time::Duration {
    let jitter = avg / 4; // 25% jitter
    let start = avg - jitter;
    let end = avg + jitter;
    let mut rgen = rand::thread_rng();
    let range = core::ops::Range { start, end };
    rgen.gen_range(range)
}
