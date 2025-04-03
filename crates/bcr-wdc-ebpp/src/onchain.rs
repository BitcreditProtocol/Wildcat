// ----- standard library imports
use std::sync::{Arc, Mutex};
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bdk_esplora::EsploraAsyncExt;
use bdk_wallet::{
    bitcoin::{bip32::Xpriv, hashes::Hash, Network},
    descriptor::template::Bip84,
    keys::{bip39::Mnemonic, DerivableKey, ExtendedKey},
    miniscript::{descriptor::KeyMap, Descriptor, DescriptorPublicKey},
    rusqlite::OpenFlags,
    KeychainKind,
};
use futures::future::JoinAll;
use rand::Rng;
// ----- local imports
use crate::error::{Error, Result};
use crate::service::OnChainWallet;

// ----- end imports

pub type BdkWallet = bdk_wallet::PersistedWallet<bdk_wallet::rusqlite::Connection>;
pub type PersistedBdkWallet = (BdkWallet, bdk_wallet::rusqlite::Connection);
pub type SingleKeyWallet = (Descriptor<DescriptorPublicKey>, KeyMap);

#[derive(Debug, Clone, serde::Deserialize)]
pub struct WalletConfig {
    mnemonic: Mnemonic,
    network: Network,
    store_path: std::path::PathBuf,
    stop_gap: usize,
    update_interval: chrono::Duration,
}

#[async_trait]
pub trait PrivateKeysRepository {
    async fn get_private_keys(&self) -> Result<Vec<SingleKeyWallet>>;
    async fn add_key(&self, key: SingleKeyWallet) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct Wallet<KeyRepo, Syncer> {
    main: Arc<Mutex<PersistedBdkWallet>>,
    // each wallet has its own updating loop task
    // the vector is mutating as keys are added and removed
    onetimes: Arc<Mutex<Vec<Arc<Mutex<PersistedBdkWallet>>>>>,
    store_path: std::path::PathBuf,
    update_interval: core::time::Duration,
    repo: KeyRepo,
    syncer: Syncer,
}

impl<KeyRepo, Syncer> Wallet<KeyRepo, Syncer>
where
    KeyRepo: PrivateKeysRepository,
    Syncer: EsploraAsyncExt + Clone + Send + Sync + 'static,
{
    const MAIN_STORE_FNAME: &'static str = "main.sqlite";

    pub async fn new(cfg: WalletConfig, repo: KeyRepo, syncer: Syncer) -> Result<Self> {
        if !cfg.store_path.is_dir() {
            return Err(Error::OnChainStore(cfg.store_path));
        }

        let update_interval = cfg.update_interval.to_std().map_err(Error::Chrono)?;

        let exkey: ExtendedKey = cfg.mnemonic.into_extended_key().map_err(Error::BDKKey)?;
        let xpriv = exkey.into_xprv(cfg.network).ok_or(Error::MnemonicToXpriv)?;
        let main_store = cfg.store_path.join(Self::MAIN_STORE_FNAME);
        let main = initialize_main_wallet(
            &main_store,
            xpriv,
            cfg.network,
            syncer.clone(),
            cfg.stop_gap,
        )
        .await?;
        let main = Arc::new(Mutex::new(main));

        let interval = random_update_interval(update_interval);
        tokio::spawn(wallet_update_loop(main.clone(), syncer.clone(), interval));

        let keys = repo.get_private_keys().await?;
        let cloned = syncer.clone();
        let joined: JoinAll<_> = keys
            .into_iter()
            .map(|key| async {
                let wlt = initialize_single_wallet(
                    &cfg.store_path,
                    key,
                    cfg.network,
                    cloned.clone(),
                    cfg.stop_gap,
                )
                .await?;
                let wlt = Arc::new(Mutex::new(wlt));
                let interval = random_update_interval(update_interval);
                tokio::spawn(wallet_update_loop(wlt.clone(), syncer.clone(), interval));
                Ok(wlt)
            })
            .collect();
        let onetimes: Vec<Arc<Mutex<PersistedBdkWallet>>> =
            joined.await.into_iter().collect::<Result<_>>()?;

        Ok(Self {
            main,
            onetimes: Arc::new(Mutex::new(onetimes)),
            repo,
            update_interval,
            store_path: cfg.store_path,
            syncer,
        })
    }

    pub async fn add_secret_key(&self, key: SingleKeyWallet) -> Result<()> {
        let network = {
            let locked = self.main.lock().unwrap();
            let (wlt, _) = &*locked;
            wlt.network()
        };
        self.repo.add_key(key.clone()).await?;
        let wlt = initialize_single_wallet(
            self.store_path.as_path(),
            key,
            network,
            self.syncer.clone(),
            1,
        )
        .await?;
        let wlt = Arc::new(Mutex::new(wlt));
        let interval = random_update_interval(self.update_interval);
        tokio::spawn(wallet_update_loop(
            wlt.clone(),
            self.syncer.clone(),
            interval,
        ));
        let mut locked = self.onetimes.lock().unwrap();
        locked.push(wlt);
        Ok(())
    }
}

#[async_trait]
impl<KeyRepo, Syncer> OnChainWallet for Wallet<KeyRepo, Syncer>
where
    KeyRepo: Sync,
    Syncer: Sync,
{
    async fn new_payment_request(&self, amount: bitcoin::Amount) -> Result<bip21::Uri> {
        let mut locked = self.main.lock().unwrap();
        let (wlt, db) = &mut *locked;
        let address = wlt.reveal_next_address(KeychainKind::External);
        wlt.persist(db).map_err(Error::BDKSQLite)?;
        let mut uri = bip21::Uri::new(address.address);
        uri.amount = Some(amount);
        Ok(uri)
    }

    async fn balance(&self) -> Result<bdk_wallet::Balance> {
        let mut balance = {
            let locked = self.main.lock().unwrap();
            let (wlt, _) = &*locked;
            wlt.balance()
        };
        let locked = self.onetimes.lock().unwrap();
        for wlt in locked.iter() {
            let locked = wlt.lock().unwrap();
            let (wlt, _) = &*locked;
            balance = balance + wlt.balance();
        }
        Ok(balance)
    }
}

async fn initialize_main_wallet<Syncer: EsploraAsyncExt + Sync>(
    store_file: &std::path::Path,
    xpriv: Xpriv,
    network: Network,
    syncer: Syncer,
    stop_gap: usize,
) -> Result<PersistedBdkWallet> {
    let internal = Bip84(xpriv, KeychainKind::Internal);
    let external = Bip84(xpriv, KeychainKind::External);

    let pre_existed = store_file.exists();
    let mut conn = bdk_wallet::rusqlite::Connection::open_with_flags(
        store_file,
        OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_READ_WRITE,
    )
    .map_err(Error::BDKSQLite)?;
    if pre_existed {
        let mut wallet = bdk_wallet::LoadParams::new()
            .descriptor(KeychainKind::Internal, Some(internal))
            .descriptor(KeychainKind::External, Some(external))
            .extract_keys()
            .check_network(network)
            .load_wallet(&mut conn)
            .map_err(Error::BDKLoadWithPersisted)?
            .ok_or_else(|| Error::BDKEmptyOption(String::from("load_wallet")))?;
        let request = wallet.start_sync_with_revealed_spks();
        let update = syncer
            .sync(request, 1)
            .await
            .map_err(|e| Error::EsploraSync(anyhow!(e)))?;
        wallet
            .apply_update(update)
            .map_err(Error::BDKCannotConnect)?;
        wallet.persist(&mut conn).map_err(Error::BDKSQLite)?;
        Ok((wallet, conn))
    } else {
        let mut wallet = bdk_wallet::CreateParams::new(external, internal)
            .network(network)
            .create_wallet(&mut conn)
            .map_err(Error::BDKCreateWithPersisted)?;
        let request = wallet.start_full_scan();
        let result = syncer
            .full_scan(request, stop_gap, 1)
            .await
            .map_err(|e| Error::EsploraFullScan(anyhow!(e)))?;
        wallet
            .apply_update(result)
            .map_err(Error::BDKCannotConnect)?;
        wallet.persist(&mut conn).map_err(Error::BDKSQLite)?;
        Ok((wallet, conn))
    }
}

async fn initialize_single_wallet<Syncer: EsploraAsyncExt + Sync>(
    store_path: &std::path::Path,
    (desc, kmap): SingleKeyWallet,
    network: Network,
    syncer: Syncer,
    stop_gap: usize,
) -> Result<PersistedBdkWallet> {
    let fname =
        bitcoin::hashes::sha256::Hash::hash(desc.to_string().as_bytes()).to_string() + ".sqlite";
    let store = store_path.join(fname);
    let pre_existed = store.exists();
    let mut conn = bdk_wallet::rusqlite::Connection::open_with_flags(
        store,
        OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_READ_WRITE,
    )
    .map_err(Error::BDKSQLite)?;
    if pre_existed {
        let mut wallet = bdk_wallet::LoadParams::new()
            .descriptor(KeychainKind::External, Some((desc, kmap)))
            .extract_keys()
            .check_network(network)
            .load_wallet(&mut conn)
            .map_err(Error::BDKLoadWithPersisted)?
            .ok_or_else(|| Error::BDKEmptyOption(String::from("load_wallet")))?;
        let request = wallet.start_sync_with_revealed_spks();
        let result = syncer
            .sync(request, 1)
            .await
            .map_err(|e| Error::EsploraFullScan(anyhow!(e)))?;
        wallet
            .apply_update(result)
            .map_err(Error::BDKCannotConnect)?;
        wallet.persist(&mut conn).map_err(Error::BDKSQLite)?;
        Ok((wallet, conn))
    } else {
        let mut wallet = bdk_wallet::CreateParams::new_single((desc, kmap))
            .network(network)
            .create_wallet(&mut conn)
            .map_err(Error::BDKCreateWithPersisted)?;
        let request = wallet.start_full_scan();
        let result = syncer
            .full_scan(request, stop_gap, 1)
            .await
            .map_err(|e| Error::EsploraFullScan(anyhow!(e)))?;
        wallet
            .apply_update(result)
            .map_err(Error::BDKCannotConnect)?;
        wallet.persist(&mut conn).map_err(Error::BDKSQLite)?;
        Ok((wallet, conn))
    }
}

async fn wallet_update_loop<Syncer: EsploraAsyncExt>(
    wlt: Arc<Mutex<PersistedBdkWallet>>,
    syncer: Syncer,
    pause: core::time::Duration,
) {
    loop {
        tokio::time::sleep(pause).await;
        let request = {
            let mut locked = wlt.lock().unwrap();
            let (wallet, _) = &mut *locked;
            wallet.start_sync_with_revealed_spks()
        };
        let result = syncer.sync(request, 1).await;
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

// random interval to spread out the load on the syncer
// given the average interval, we spread it by +/- 25%
fn random_update_interval(avg: core::time::Duration) -> core::time::Duration {
    let jitter = avg / 4; // 25% jitter
    let start = avg - jitter;
    let end = avg + jitter;
    let mut rgen = rand::thread_rng();
    let range = core::ops::Range { start, end };
    rgen.gen_range(range)
}
