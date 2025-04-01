// ----- standard library imports
use std::sync::{Arc, Mutex};
// ----- extra library imports
use async_trait::async_trait;
use bdk_wallet::{
    bitcoin::Network,
    descriptor::template::Bip84,
    keys::{bip39::Mnemonic, DerivableKey, ExtendedKey},
    rusqlite::OpenFlags,
    KeychainKind,
};
// ----- local imports
use crate::error::{Error, Result};
use crate::service::Bip39Wallet;

// ----- end imports

pub type BdkWallet = bdk_wallet::PersistedWallet<bdk_wallet::rusqlite::Connection>;
pub type PersistedBdkWallet = (BdkWallet, bdk_wallet::rusqlite::Connection);

#[derive(Debug, Clone, serde::Deserialize)]
pub struct WalletConfig {
    mnemonic: Mnemonic,
    network: Network,
    main_store: std::path::PathBuf,
}

#[derive(Debug, Clone)]
pub struct Wallet {
    inner: Arc<Mutex<PersistedBdkWallet>>,
}

impl Wallet {
    pub fn new(cfg: WalletConfig) -> Result<Self> {
        let exkey: ExtendedKey = cfg.mnemonic.into_extended_key().map_err(Error::BDKKey)?;
        let xpriv = exkey.into_xprv(cfg.network).ok_or(Error::MnemonicToXpriv)?;
        let internal = Bip84(xpriv, KeychainKind::Internal);
        let external = Bip84(xpriv, KeychainKind::External);

        let pre_existed = cfg.main_store.exists();
        let mut conn = bdk_wallet::rusqlite::Connection::open_with_flags(
            &cfg.main_store,
            OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(Error::BDKSQLite)?;
        let wallet = if pre_existed {
            bdk_wallet::LoadParams::new()
                .descriptor(KeychainKind::Internal, Some(internal))
                .descriptor(KeychainKind::External, Some(external))
                .extract_keys()
                .check_network(cfg.network)
                .load_wallet(&mut conn)
                .map_err(Error::BDKLoadWithPersisted)?
                .ok_or_else(|| Error::BDKEmptyOption(String::from("load_wallet")))?
        } else {
            bdk_wallet::CreateParams::new(external, internal)
                .network(cfg.network)
                .create_wallet(&mut conn)
                .map_err(Error::BDKCreateWithPersisted)?
        };
        Ok(Self {
            inner: Arc::new(Mutex::new((wallet, conn))),
        })
    }
}

#[async_trait]
impl Bip39Wallet for Wallet {
    async fn new_payment_request(&self, amount: bitcoin::Amount) -> Result<bip21::Uri> {
        let mut locked = self.inner.lock().unwrap();
        let (wlt, db) = &mut *locked;
        let address = wlt.reveal_next_address(KeychainKind::External);
        wlt.persist(db).map_err(Error::BDKSQLite)?;
        let mut uri = bip21::Uri::new(address.address);
        uri.amount = Some(amount);
        Ok(uri)
    }

    async fn balance(&self) -> Result<bdk_wallet::Balance> {
        let locked = self.inner.lock().unwrap();
        let (wlt, _) = &*locked;
        Ok(wlt.balance())
    }
}
