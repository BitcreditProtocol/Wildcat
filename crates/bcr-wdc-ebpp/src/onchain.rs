// ----- standard library imports
use std::sync::{Arc, Mutex, MutexGuard};
// ----- extra library imports
use async_trait::async_trait;
use bdk_core::bitcoin::Amount;
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
    miniscript::{descriptor::KeyMap, Descriptor, DescriptorPublicKey},
    rusqlite::OpenFlags,
    KeychainKind, SignOptions, TxOrdering,
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
    network: Network,
    store_path: std::path::PathBuf,
    stop_gap: usize,
    max_confirmation_blocks: usize,
    avg_transaction_size_bytes: usize,
}

#[async_trait]
pub trait PrivateKeysRepository {
    async fn get_private_keys(&self) -> Result<Vec<SingleSecretKeyDescriptor>>;
    async fn add_key(&self, key: SingleSecretKeyDescriptor) -> Result<()>;
}

#[derive(Debug)]
pub struct Wallet<KeyRepo, ElectrumApi> {
    main: Arc<Mutex<PersistedBdkWallet>>,
    // the vector is mutating as keys are added and removed
    singles: Arc<Mutex<Vec<Arc<Mutex<PersistedBdkWallet>>>>>,
    store_path: std::path::PathBuf,
    repo: KeyRepo,
    electrum_client: Arc<BdkElectrumClient<ElectrumApi>>,
    // configs
    network: Network,
    stop_gap: usize,                   // number of unused addresses to stop scanning
    max_confirmation_blocks: usize,    // number of blocks to confirm a transaction
    avg_transaction_size_bytes: usize, // transaction size in bytes used to estimate fees
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
            singles: self.singles.clone(),
            store_path: self.store_path.clone(),
            repo: self.repo.clone(),
            electrum_client: self.electrum_client.clone(),
            network: self.network,
            stop_gap: self.stop_gap,
            max_confirmation_blocks: self.max_confirmation_blocks,
            avg_transaction_size_bytes: self.avg_transaction_size_bytes,
        }
    }
}

impl<KeyRepo, ElectrumApi> Wallet<KeyRepo, ElectrumApi>
where
    KeyRepo: PrivateKeysRepository,
    ElectrumApi: electrum_client::ElectrumApi + Send + Sync + 'static,
{
    const MAIN_STORE_FNAME: &'static str = "main.sqlite";
    const MIN_FEE_RATE_BTC_PER_KBYTE: f64 = 0.000005; // minimum fee rate in btc/KByte
    const BATCH_SIZE: usize = 15;

    pub async fn new(
        seed: &[u8],
        cfg: WalletConfig,
        repo: KeyRepo,
        api: ElectrumApi,
    ) -> Result<Self> {
        if !cfg.store_path.is_dir() {
            return Err(Error::OnChainStore(cfg.store_path));
        }

        let electrum_client = Arc::new(BdkElectrumClient::new(api));

        let xpriv = Xpriv::new_master(cfg.network, seed).map_err(Error::BTCBIP32)?;
        let main_store = cfg.store_path.join(Self::MAIN_STORE_FNAME);
        let (age, main) = initialize_main_wallet(&main_store, xpriv, cfg.network)?;
        let main = Arc::new(Mutex::new(main));
        let cloned_main = main.clone();
        let cloned_electrum_client = electrum_client.clone();
        let stop_gap = cfg.stop_gap;
        let batch_size = Self::BATCH_SIZE;
        timed_spawn_blocking("Wallet::new, full scan for main wallet", move || {
            wallet_full_scan(
                cloned_main,
                age,
                cloned_electrum_client,
                stop_gap,
                batch_size,
            );
        })
        .await?;

        let mut singles = Vec::new();
        let keys = repo.get_private_keys().await?;
        for key in keys {
            let (age, wlt) = initialize_single_wallet(&cfg.store_path, key, cfg.network)?;
            let wlt = Arc::new(Mutex::new(wlt));
            let cloned_wlt = main.clone();
            let cloned_electrum_client = electrum_client.clone();
            let stop_gap = cfg.stop_gap;
            let batch_size = Self::BATCH_SIZE;
            timed_spawn_blocking("Wallet::new, full scan for single-use wallet", move || {
                wallet_full_scan(
                    cloned_wlt,
                    age,
                    cloned_electrum_client,
                    stop_gap,
                    batch_size,
                );
            })
            .await?;
            singles.push(wlt);
        }

        Ok(Self {
            main,
            singles: Arc::new(Mutex::new(singles)),
            repo,
            store_path: cfg.store_path,
            electrum_client,
            network: cfg.network,
            stop_gap: cfg.stop_gap,
            max_confirmation_blocks: cfg.max_confirmation_blocks,
            avg_transaction_size_bytes: cfg.avg_transaction_size_bytes,
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

    fn network(&self) -> bdk_wallet::bitcoin::Network {
        self.network
    }

    async fn balance(&self) -> Result<bdk_wallet::Balance> {
        wallets_sync(
            self.main.clone(),
            self.singles.clone(),
            self.electrum_client.clone(),
        )
        .await?;
        let mut balance = {
            let locked = self.main.lock().unwrap();
            let (wlt, _) = &*locked;
            wlt.balance()
        };
        let locked_singles = self.singles.lock().unwrap();
        for wlt in locked_singles.iter() {
            let locked = wlt.lock().unwrap();
            let (wlt, _) = &*locked;
            balance = balance + wlt.balance();
        }
        Ok(balance)
    }

    async fn add_descriptor(&self, descriptor: &str) -> Result<btc::Address> {
        wallets_sync(
            self.main.clone(),
            self.singles.clone(),
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
        let batch_size = Self::BATCH_SIZE;
        timed_spawn_blocking("Wallet::add_descriptor, new single full_scan", move || {
            wallet_full_scan(
                cloned_wlt,
                age,
                cloned_electrum_client,
                stop_gap,
                batch_size,
            );
        })
        .await?;
        let mut locked = self.singles.lock().unwrap();
        locked.push(wlt);
        Ok(addr_info.address)
    }

    async fn get_address_balance(&self, addr: &btc::Address) -> Result<btc::Amount> {
        wallets_sync(
            self.main.clone(),
            self.singles.clone(),
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
            let locked = self.singles.lock().unwrap();
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
    async fn estimate_fees(&self) -> Result<btc::Amount> {
        let cloned = self.electrum_client.clone();
        let max_confirmation_blocks = self.max_confirmation_blocks;
        let avg_transaction_size = self.avg_transaction_size_bytes;
        let raw_fee_rate: f64 = timed_spawn_blocking("Wallet::estimate_fees", move || {
            cloned.inner.estimate_fee(max_confirmation_blocks)
        })
        .await??;
        let fee_rate = if raw_fee_rate > Self::MIN_FEE_RATE_BTC_PER_KBYTE {
            raw_fee_rate
        } else {
            Self::MIN_FEE_RATE_BTC_PER_KBYTE
        };
        let fee = fee_rate * avg_transaction_size as f64 / 1000.0;
        let amount = Amount::from_btc(fee)?;
        Ok(amount)
    }

    async fn send_to(
        &self,
        recipient: btc::Address,
        amount: btc::Amount,
        max_fee: btc::Amount,
    ) -> Result<(btc::Txid, btc::Amount)> {
        wallets_sync(
            self.main.clone(),
            self.singles.clone(),
            self.electrum_client.clone(),
        )
        .await?;
        // 1. send from a single wallet whose balance is greater than amount + max_fee
        {
            let locked_singles = self.singles.lock().unwrap();
            for wlt in locked_singles.iter() {
                let locked = wlt.lock().unwrap();
                if locked.0.balance().confirmed > amount + max_fee {
                    let sweeping_address = {
                        let mut locked = self.main.lock().unwrap();
                        let (wlt, db) = &mut *locked;
                        let address_info = wlt.reveal_next_address(KeychainKind::External);
                        wlt.persist(db)?;
                        address_info.address
                    };
                    return sweep_to(
                        self.electrum_client.clone(),
                        locked,
                        recipient,
                        sweeping_address,
                        amount,
                        max_fee,
                    );
                }
            }
        }
        // 2. send from the main wallet
        let main_locked = self.main.lock().unwrap();
        return send_to(
            self.electrum_client.clone(),
            main_locked,
            recipient,
            amount,
            max_fee,
        );
    }

    async fn is_confirmed(&self, tx_id: btc::Txid) -> Result<bool> {
        wallets_sync(
            self.main.clone(),
            self.singles.clone(),
            self.electrum_client.clone(),
        )
        .await?;
        {
            let locked_singles = self.singles.lock().unwrap();
            for single in locked_singles.iter() {
                let locked = single.lock().unwrap();
                let (wlt, _) = &*locked;
                if let Some(tx) = wlt.get_tx(tx_id) {
                    return Ok(tx.chain_position.is_confirmed());
                }
            }
        }
        let locked = self.main.lock().unwrap();
        let (wlt, _) = &*locked;
        if let Some(tx) = wlt.get_tx(tx_id) {
            return Ok(tx.chain_position.is_confirmed());
        }
        Err(Error::TxNotFound(tx_id))
    }
}

#[derive(Debug, PartialEq)]
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
    let wallet_opt = bdk_wallet::Wallet::load()
        .descriptor(KeychainKind::Internal, Some(internal.clone()))
        .descriptor(KeychainKind::External, Some(external.clone()))
        .extract_keys()
        .check_network(network)
        .load_wallet(&mut conn)?;
    match wallet_opt {
        Some(wallet) => Ok((WalletAge::Old, (wallet, conn))),
        None => {
            let wallet = bdk_wallet::Wallet::create(external, internal)
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
    let wallet_opt = bdk_wallet::Wallet::load()
        .descriptor(KeychainKind::External, Some((desc.clone(), kmap.clone())))
        .extract_keys()
        .check_network(network)
        .load_wallet(&mut conn)?;
    match wallet_opt {
        Some(wallet) => Ok((WalletAge::Old, (wallet, conn))),
        None => {
            let wallet = bdk_wallet::Wallet::create_single((desc, kmap))
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
    batch_size: usize,
) where
    ElectrumApi: electrum_client::ElectrumApi,
{
    if matches!(age, WalletAge::New) {
        let request = {
            let mut locked = wlt.lock().unwrap();
            let (wallet, _) = &mut *locked;
            wallet.start_full_scan().build()
        };
        let result = electrum_client.full_scan(request, stop_gap, batch_size, false);
        if result.is_err() {
            tracing::error!("full scan error: {}", result.unwrap_err());
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
    singles: Arc<Mutex<Vec<Arc<Mutex<PersistedBdkWallet>>>>>,
    electrum_client: Arc<BdkElectrumClient<ElectrumApi>>,
) -> Result<()>
where
    ElectrumApi: electrum_client::ElectrumApi + Send + Sync + 'static,
{
    let cloned_electrum_client = electrum_client.clone();
    let main_sync_task = timed_spawn_blocking("wallets_sync, main wallet ", || {
        wallet_sync(main, cloned_electrum_client)
    });
    let joined: JoinAll<_> = {
        let locked_singles = singles.lock().unwrap();
        locked_singles
            .iter()
            .map(|wlt| {
                let cloned_wlt = wlt.clone();
                let cloned_electrum_client = electrum_client.clone();
                timed_spawn_blocking("wallets_sync, single wallet", move || {
                    wallet_sync(cloned_wlt, cloned_electrum_client)
                })
            })
            .collect()
    };

    main_sync_task.await??;
    for task in joined.await {
        task??;
    }
    Ok(())
}

// blocking function as we need to keep the wallet locked
fn sweep_to<ElectrumApi>(
    electrum_client: Arc<BdkElectrumClient<ElectrumApi>>,
    mut locked_wlt: MutexGuard<PersistedBdkWallet>,
    recipient: btc::Address,
    sweeping_address: btc::Address,
    amount: btc::Amount,
    max_fee: btc::Amount,
) -> Result<(btc::Txid, btc::Amount)>
where
    ElectrumApi: electrum_client::ElectrumApi,
{
    let (wallet, db) = &mut *locked_wlt;
    let mut psbt = {
        let mut builder = wallet.build_tx();
        builder
            .ordering(TxOrdering::Untouched)
            .add_recipient(recipient.script_pubkey(), amount)
            .drain_wallet()
            .drain_to(sweeping_address.script_pubkey())
            .fee_absolute(max_fee);
        builder.finish().map_err(Error::BDKCreateTx)?
    };
    let signopt = SignOptions::default();
    let signok = wallet
        .sign(&mut psbt, signopt.clone())
        .map_err(Error::BDKSigner)?;
    if !signok {
        return Err(Error::BDKSignOpNotOK);
    }
    let finalok = wallet
        .finalize_psbt(&mut psbt, signopt)
        .map_err(Error::BDKSigner)?;
    if !finalok {
        return Err(Error::BDKSignOpNotOK);
    }
    let total_fee = psbt.fee()?;
    let tx = psbt.extract_tx()?;
    let txid = electrum_client.transaction_broadcast(&tx)?;
    wallet.persist(db)?;
    let total_spent = amount + total_fee;
    Ok((txid, total_spent))
}

// blocking function as we need to keep the wallet locked
fn send_to<ElectrumApi>(
    electrum_client: Arc<BdkElectrumClient<ElectrumApi>>,
    mut locked_wlt: MutexGuard<PersistedBdkWallet>,
    recipient: btc::Address,
    amount: btc::Amount,
    max_fee: btc::Amount,
) -> Result<(btc::Txid, btc::Amount)>
where
    ElectrumApi: electrum_client::ElectrumApi,
{
    let (wallet, db) = &mut *locked_wlt;
    let mut psbt = {
        let mut builder = wallet.build_tx();
        builder
            .ordering(TxOrdering::Untouched)
            .add_recipient(recipient.script_pubkey(), amount)
            .fee_absolute(max_fee);
        builder.finish().map_err(Error::BDKCreateTx)?
    };
    let signopt = SignOptions::default();
    let signok = wallet
        .sign(&mut psbt, signopt.clone())
        .map_err(Error::BDKSigner)?;
    if !signok {
        return Err(Error::BDKSignOpNotOK);
    }
    let finalok = wallet
        .finalize_psbt(&mut psbt, signopt)
        .map_err(Error::BDKSigner)?;
    if !finalok {
        return Err(Error::BDKSignOpNotOK);
    }
    let total_fee = psbt.fee()?;
    let tx = psbt.extract_tx()?;
    let txid = electrum_client.transaction_broadcast(&tx)?;
    let total_spent = amount + total_fee;
    wallet.persist(db)?;
    Ok((txid, total_spent))
}

async fn timed_spawn_blocking<F, T>(
    note: &'static str,
    f: F,
) -> std::result::Result<T, tokio::task::JoinError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let hndl = tokio::task::spawn_blocking(move || {
        let start = std::time::Instant::now();
        let result = f();
        let end = std::time::Instant::now();
        let elapsed = end - start;
        tracing::trace!("{note} took: {elapsed:?}");
        Ok(result)
    });
    hndl.await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn initialize_main_wallet_new() {
        //iancoleman.io
        let xpriv = btc::bip32::Xpriv::from_str("tprv8ZgxMBicQKsPdehyjwGnn4uRdK3oQZbRAuTi3TteaewxNLjWyxdZwmvYvpPSDEhJUtup7ndWFhK2pxB11ezuBoCzzKddCSdgikVAAUfWe8u").unwrap();
        // derivation m/84'/1'/0'
        let expected_external_derivation_path = "tpubDDJ48Eu2muRuDwQ3F31v8cctLJRCaXzTQHTrBxmk4pqvHos8L7genWReesxbcCuzu4RStW65GCDFrUGqWAnRWA5qbYFuC2vREvRtPYsL1rp";

        let tmp_dir = tempfile::tempdir().unwrap();
        let store_file = tmp_dir.path().join("main.sqlite");
        let network = btc::Network::Testnet;

        let (age, wlt) = initialize_main_wallet(&store_file, xpriv, network).unwrap();
        assert_eq!(age, WalletAge::New);
        let external_desc = wlt.0.public_descriptor(bdk_wallet::KeychainKind::External);
        assert!(external_desc
            .to_string()
            .contains(expected_external_derivation_path));
    }

    #[test]
    fn initialize_single_wallet_new() {
        // learnmeabitcoin.com/technical/keys/private-key/wif/
        // learnmeabitcoin.com/technical/keys/
        let descriptor = "wpkh(cQv7zQXyJ36AsbCqow8NwNp7QXyFG7bXcPfdEhnP7QmZx4S5S9Lw)";
        let expected_publickey_compressed =
            "03e416ffcdde3d5b83a1940fda7aec1179ed3382d080da9c2672c02115a37ad45e";
        let (descr, keymap) =
            Descriptor::parse_descriptor(secp256k1::global::SECP256K1, descriptor).unwrap();

        let tmp_dir = tempfile::tempdir().unwrap();
        let network = btc::Network::Testnet;

        let (age, wlt) =
            initialize_single_wallet(tmp_dir.path(), (descr, keymap), network).unwrap();
        assert_eq!(age, WalletAge::New);
        let external_desc = wlt.0.public_descriptor(bdk_wallet::KeychainKind::External);
        assert!(external_desc
            .to_string()
            .contains(expected_publickey_compressed));
    }
}
