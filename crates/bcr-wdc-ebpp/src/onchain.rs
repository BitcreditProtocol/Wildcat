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
    chain::ChainPosition,
    descriptor::template::Bip84,
    keys::{bip39::Mnemonic, DerivableKey, ExtendedKey},
    miniscript::{descriptor::KeyMap, Descriptor, DescriptorPublicKey},
    rusqlite::OpenFlags,
    KeychainKind,
};
use futures::future::JoinAll;
use serde_with::serde_as;
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
}

#[async_trait]
pub trait PrivateKeysRepository {
    async fn get_private_keys(&self) -> Result<Vec<SingleSecretKeyDescriptor>>;
    async fn add_key(&self, key: SingleSecretKeyDescriptor) -> Result<()>;
}

#[derive(Debug)]
pub struct Wallet<KeyRepo, ElectrumApi> {
    main: Arc<Mutex<PersistedBdkWallet>>,
    // each wallet has its own updating loop task
    // the vector is mutating as keys are added and removed
    onetimes: Arc<Mutex<Vec<Arc<Mutex<PersistedBdkWallet>>>>>,
    store_path: std::path::PathBuf,
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

        let electrum_client = Arc::new(BdkElectrumClient::new(api));

        let exkey: ExtendedKey = cfg.mnemonic.into_extended_key()?;
        let xpriv = exkey.into_xprv(cfg.network).ok_or(Error::MnemonicToXpriv)?;
        let main_store = cfg.store_path.join(Self::MAIN_STORE_FNAME);
        let (age, main) = initialize_main_wallet(&main_store, xpriv, cfg.network)?;
        let main = Arc::new(Mutex::new(main));
        let cloned_main = main.clone();
        let cloned_electrum_client = electrum_client.clone();
        let stop_gap = cfg.stop_gap;
        tokio::task::spawn_blocking(move || {
            wallet_full_scan(cloned_main, age, cloned_electrum_client, stop_gap);
        });

        let mut onetimes = Vec::new();
        let keys = repo.get_private_keys().await?;
        for key in keys {
            let (age, wlt) = initialize_single_wallet(&cfg.store_path, key, cfg.network)?;
            let wlt = Arc::new(Mutex::new(wlt));
            let cloned_wlt = main.clone();
            let cloned_electrum_client = electrum_client.clone();
            let stop_gap = cfg.stop_gap;
            tokio::task::spawn_blocking(move || {
                wallet_full_scan(cloned_wlt, age, cloned_electrum_client, stop_gap);
            });
            onetimes.push(wlt);
        }

        Ok(Self {
            main,
            onetimes: Arc::new(Mutex::new(onetimes)),
            repo,
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
        let mut locked = self.main.lock().unwrap();
        let (wlt, db) = &mut *locked;
        let address_info = wlt.reveal_next_address(KeychainKind::External);
        wlt.persist(db)?;
        Ok(address_info.address)
    }

    async fn balance(&self) -> Result<bdk_wallet::Balance> {
        wallets_sync(
            self.main.clone(),
            self.onetimes.clone(),
            self.electrum_client.clone(),
        )
        .await?;
        let mut balance = {
            let locked = self.main.lock().unwrap();
            let (wlt, _) = &*locked;
            wlt.balance()
        };
        let locked_onetimes = self.onetimes.lock().unwrap();
        for wlt in locked_onetimes.iter() {
            let locked = wlt.lock().unwrap();
            let (wlt, _) = &*locked;
            balance = balance + wlt.balance();
        }
        Ok(balance)
    }

    async fn add_descriptor(&self, descriptor: &str) -> Result<btc::Address> {
        wallets_sync(
            self.main.clone(),
            self.onetimes.clone(),
            self.electrum_client.clone(),
        )
        .await?;
        let desc: SingleSecretKeyDescriptor = {
            let locked = self.main.lock().unwrap();
            let (wlt, _) = &*locked;
            let secp_ctx = wlt.secp_ctx();
            Descriptor::parse_descriptor(secp_ctx, descriptor)?
        };
        self.repo.add_key(desc.clone()).await?;
        let (age, mut wlt) = initialize_single_wallet(&self.store_path, desc, self.network)?;
        let addr_info = wlt.0.reveal_next_address(KeychainKind::External);
        wlt.0.persist(&mut wlt.1)?;
        let wlt = Arc::new(Mutex::new(wlt));
        let cloned_wlt = wlt.clone();
        let cloned_electrum_client = self.electrum_client.clone();
        let stop_gap = self.stop_gap;
        tokio::task::spawn_blocking(move || {
            wallet_full_scan(cloned_wlt, age, cloned_electrum_client, stop_gap);
        });
        let mut locked = self.onetimes.lock().unwrap();
        locked.push(wlt);
        Ok(addr_info.address)
    }

    async fn get_address_balance(&self, addr: &btc::Address) -> Result<btc::Amount> {
        wallets_sync(
            self.main.clone(),
            self.onetimes.clone(),
            self.electrum_client.clone(),
        )
        .await?;
        let script = addr.script_pubkey();
        {
            let locked = self.main.lock().unwrap();
            let (wlt, _) = &*locked;
            if wlt.is_mine(script.clone()) {
                let total: btc::Amount = wlt
                    .list_unspent()
                    .filter(|output| !output.is_spent)
                    .filter(|output| {
                        matches!(output.chain_position, ChainPosition::Confirmed { .. })
                    })
                    .filter(|output| output.txout.script_pubkey == script)
                    .fold(btc::Amount::ZERO, |sum, output| sum + output.txout.value);
                return Ok(total);
            }
        }
        {
            let locked = self.onetimes.lock().unwrap();
            for wlt in locked.iter() {
                let wlt_locked = wlt.lock().unwrap();
                let (wlt, _) = &*wlt_locked;
                if wlt.is_mine(script.clone()) {
                    let total: btc::Amount = wlt
                        .list_unspent()
                        .filter(|output| !output.is_spent)
                        .filter(|output| {
                            matches!(output.chain_position, ChainPosition::Confirmed { .. })
                        })
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

fn wallet_full_scan<ElectrumApi>(
    wlt: Arc<Mutex<PersistedBdkWallet>>,
    age: WalletAge,
    electrum_client: Arc<BdkElectrumClient<ElectrumApi>>,
    stop_gap: usize,
) where
    ElectrumApi: electrum_client::ElectrumApi,
{
    if matches!(age, WalletAge::New) {
        let request = {
            let mut locked = wlt.lock().unwrap();
            let (wallet, _) = &mut *locked;
            wallet.start_full_scan()
        };
        let result = electrum_client.full_scan(request, stop_gap, 1, false);
        if result.is_err() {
            log::error!("full scan error: {}", result.unwrap_err());
        }
    }
}

fn wallet_sync<ElectrumApi>(
    wlt: Arc<Mutex<PersistedBdkWallet>>,
    electrum_client: Arc<BdkElectrumClient<ElectrumApi>>,
) -> Result<()>
where
    ElectrumApi: electrum_client::ElectrumApi,
{
    let request = {
        let mut locked = wlt.lock().unwrap();
        let (wallet, _) = &mut *locked;
        wallet.start_sync_with_revealed_spks()
    };
    let update = electrum_client
        .sync(request, 1, false)
        .map_err(Error::Electrum)?;
    let mut locked = wlt.lock().unwrap();
    let (wallet, db) = &mut *locked;
    wallet.apply_update(update).unwrap();
    wallet.persist(db).unwrap();
    Ok(())
}

async fn wallets_sync<ElectrumApi>(
    main: Arc<Mutex<PersistedBdkWallet>>,
    onetimes: Arc<Mutex<Vec<Arc<Mutex<PersistedBdkWallet>>>>>,
    electrum_client: Arc<BdkElectrumClient<ElectrumApi>>,
) -> Result<()>
where
    ElectrumApi: electrum_client::ElectrumApi + Send + Sync + 'static,
{
    let cloned_electrum_client = electrum_client.clone();
    let main_sync_task = tokio::task::spawn_blocking(|| wallet_sync(main, cloned_electrum_client));
    let joined: JoinAll<_> = {
        let locked_onetimes = onetimes.lock().unwrap();
        locked_onetimes
            .iter()
            .map(|wlt| {
                let cloned_wlt = wlt.clone();
                let cloned_electrum_client = electrum_client.clone();
                tokio::task::spawn_blocking(move || wallet_sync(cloned_wlt, cloned_electrum_client))
            })
            .collect()
    };

    main_sync_task.await??;
    for task in joined.await {
        task??;
    }
    Ok(())
}
