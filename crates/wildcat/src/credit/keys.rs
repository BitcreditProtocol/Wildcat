// ----- standard library imports
// ----- extra library imports
use anyhow::{Error as AnyError, Result as AnyResult};
use async_trait::async_trait;
use bcr_wdc_keys as keys;
use bcr_wdc_keys::KeysetID;
use bitcoin::bip32 as btc32;
use cdk::nuts::nut00 as cdk00;
use cdk::nuts::nut01 as cdk01;
use cdk::nuts::nut02 as cdk02;
use thiserror::Error;
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::credit::quotes::KeyFactory;
use crate::swap;
use crate::TStamp;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("cdk::nut01 error {0}")]
    CdkNut01(#[from] cdk01::Error),
    #[error("repository error {0}")]
    Repository(#[from] AnyError),
}

// ---------- required traits
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait QuoteBasedRepository: Send + Sync {
    async fn load(&self, kid: &KeysetID, qid: Uuid) -> AnyResult<Option<keys::KeysetEntry>>;
    async fn store(
        &self,
        qid: Uuid,
        keyset: cdk02::MintKeySet,
        info: cdk::mint::MintKeySetInfo,
    ) -> AnyResult<()>;
}

// ---------- Keys Factory
#[derive(Clone)]
pub struct Factory<QuoteKeys, MaturityKeys> {
    ctx: bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>,
    xpriv: btc32::Xpriv,
    quote_keys: QuoteKeys,
    maturing_keys: MaturityKeys,
    unit: cdk00::CurrencyUnit,
}

impl<QuoteKeys, MaturityKeys> Factory<QuoteKeys, MaturityKeys> {
    pub const MAX_ORDER: u8 = 20;
    pub const CURRENCY_UNIT: &'static str = "crsat";

    pub fn new(seed: &[u8], quote_keys: QuoteKeys, maturing_keys: MaturityKeys) -> Self {
        Self {
            ctx: bitcoin::secp256k1::Secp256k1::new(),
            xpriv: btc32::Xpriv::new_master(bitcoin::Network::Bitcoin, seed).expect("bitcoin FAIL"),
            quote_keys,
            maturing_keys,
            unit: cdk00::CurrencyUnit::Custom(String::from(Self::CURRENCY_UNIT)),
        }
    }
}

#[async_trait]
impl<QuoteKeys, MaturityKeys> KeyFactory for Factory<QuoteKeys, MaturityKeys>
where
    QuoteKeys: QuoteBasedRepository,
    MaturityKeys: keys::Repository,
{
    async fn generate(
        &self,
        keysetid: KeysetID,
        quote: uuid::Uuid,
        bill_maturity_date: TStamp,
    ) -> AnyResult<cdk02::MintKeySet> {
        let path = keys::generate_keyset_path(keysetid, Some(quote));
        let keys = cdk02::MintKeySet::generate_from_xpriv(
            &self.ctx,
            self.xpriv,
            Self::MAX_ORDER,
            self.unit.clone(),
            path.clone(),
        )
        .keys;

        let info = cdk::mint::MintKeySetInfo {
            id: keysetid.into(),
            unit: self.unit.clone(),
            active: false,
            valid_from: chrono::Utc::now().timestamp() as u64,
            valid_to: Some(bill_maturity_date.timestamp() as u64),
            derivation_path: path,
            derivation_path_index: None,
            max_order: Self::MAX_ORDER,
            input_fee_ppk: 0,
        };
        let set = cdk02::MintKeySet {
            id: keysetid.into(),
            keys,
            unit: self.unit.clone(),
        };
        self.quote_keys.store(quote, set.clone(), info).await?;

        let kid = keys::generate_keyset_id_from_date(bill_maturity_date, 0);
        if self.maturing_keys.info(&kid).await?.is_some() {
            return Ok(set);
        }

        let kid = keys::generate_keyset_id_from_date(bill_maturity_date, 0);
        let path = keys::generate_keyset_path(kid, None);
        // adding <rotate_idx> starts from zero
        let rotate_child =
            btc32::ChildNumber::from_hardened_idx(0).expect("rotate index 0 is valid");
        let indexed_path = path.child(rotate_child);
        let mut keyset = cdk02::MintKeySet::generate_from_xpriv(
            &self.ctx,
            self.xpriv,
            Self::MAX_ORDER,
            self.unit.clone(),
            indexed_path,
        );
        keyset.id = kid.into();
        let info = cdk::mint::MintKeySetInfo {
            id: keyset.id,
            unit: self.unit.clone(),
            active: true,
            valid_from: chrono::Utc::now().timestamp() as u64,
            valid_to: Some(bill_maturity_date.timestamp() as u64),
            derivation_path: path,
            derivation_path_index: Some(0),
            max_order: Self::MAX_ORDER,
            input_fee_ppk: 0,
        };
        self.maturing_keys.store(keyset.clone(), info).await?;

        Ok(set)
    }
}

// ---------- Swap Keys Repository
#[derive(Default, Clone)]
pub struct SwapRepository<KeysRepo, ActiveRepo> {
    pub endorsed_keys: KeysRepo,
    pub maturity_keys: KeysRepo,
    pub debit_keys: ActiveRepo,
}

impl<KeysRepo, ActiveRepo> SwapRepository<KeysRepo, ActiveRepo>
where
    KeysRepo: keys::Repository,
    ActiveRepo: keys::ActiveRepository,
{
    async fn find_maturity_keys_from_maturity_date(
        &self,
        maturity_date: TStamp,
        mut rotation_idx: u32,
    ) -> Result<Option<KeysetID>> {
        let mut kid = keys::generate_keyset_id_from_date(maturity_date, rotation_idx);
        while let Some(info) = self.maturity_keys.info(&kid).await? {
            if info.active {
                return Ok(Some(kid));
            }
            rotation_idx += 1;
            kid = keys::generate_keyset_id_from_date(maturity_date, rotation_idx)
        }
        Ok(None)
    }

    async fn find_maturity_keys_from_id(&self, kid: &KeysetID) -> Result<Option<KeysetID>> {
        if let Some(info) = self.maturity_keys.info(kid).await? {
            if info.active {
                return Ok(Some(*kid));
            }
            let valid_to = info.valid_to.expect("valid_to field not set") as i64;
            let maturity =
                TStamp::from_timestamp(valid_to, 0).expect("datetime conversion from u64");
            let rotation_index = info
                .derivation_path_index
                .expect("derivation_path_index not set");
            return self
                .find_maturity_keys_from_maturity_date(maturity, rotation_index + 1)
                .await;
        }
        Ok(None)
    }
}

#[async_trait]
impl<KeysRepo, ActiveRepo> swap::KeysRepository for SwapRepository<KeysRepo, ActiveRepo>
where
    KeysRepo: keys::Repository,
    ActiveRepo: keys::ActiveRepository,
{
    async fn keyset(&self, id: &KeysetID) -> AnyResult<Option<cdk02::MintKeySet>> {
        if let Some(keyset) = self.endorsed_keys.keyset(id).await? {
            return Ok(Some(keyset));
        }
        if let Some(keyset) = self.maturity_keys.keyset(id).await? {
            return Ok(Some(keyset));
        }
        self.debit_keys.keyset(id).await
    }
    async fn info(&self, id: &KeysetID) -> AnyResult<Option<cdk::mint::MintKeySetInfo>> {
        if let Some(info) = self.endorsed_keys.info(id).await? {
            return Ok(Some(info));
        }
        if let Some(info) = self.maturity_keys.info(id).await? {
            return Ok(Some(info));
        }
        self.debit_keys.info(id).await
    }
    // in case keyset id is inactive, returns the proper replacement for it
    async fn replacing_id(&self, kid: &KeysetID) -> AnyResult<Option<KeysetID>> {
        if let Some(info) = self.endorsed_keys.info(kid).await? {
            let valid_to = info.valid_to.expect("valid_to field not set") as i64;
            let maturity =
                TStamp::from_timestamp(valid_to, 0).expect("datetime conversion from u64");
            if let Some(id) = self
                .find_maturity_keys_from_maturity_date(maturity, 0)
                .await?
            {
                return Ok(Some(id));
            }
        }
        if let Some(kid) = self.find_maturity_keys_from_id(kid).await? {
            return Ok(Some(kid));
        }
        let kid = self
            .debit_keys
            .info_active()
            .await?
            .map(|info| info.id)
            .map(KeysetID::from);
        Ok(kid)
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::keys::test_utils as keys_test;
    use crate::swap::KeysRepository;
    use mockall::predicate::*;
    use std::str::FromStr;

    #[tokio::test]
    async fn test_keys_factory_generate() {
        let seed = bip39::Mnemonic::from_str("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").unwrap().to_seed("");

        let keyid = KeysetID::from(cdk02::Id::from_bytes(&[0u8; 8]).unwrap());
        let quote = uuid::Uuid::from_u128(0);
        let maturity = chrono::DateTime::parse_from_rfc3339("2021-01-01T00:00:00Z")
            .unwrap()
            .to_utc();

        let mut maturitykeys_repo = keys_test::MockRepository::new();
        maturitykeys_repo.expect_info().returning(|_| Ok(None));
        maturitykeys_repo.expect_store().returning(|_, _| Ok(()));
        let mut quotekeys_repo = MockQuoteBasedRepository::new();
        quotekeys_repo
            .expect_store()
            .with(eq(quote), always(), always())
            .returning(|_, _, _| Ok(()));
        //quotekeys_repo.expect_store().returning(|_, _| Ok(()));

        let factory = Factory::new(&seed, quotekeys_repo, maturitykeys_repo);

        let keyset = factory.generate(keyid, quote, maturity).await.unwrap();
        // m/129372'/129534'/0'/927402239'/0'
        let key = &keyset.keys[&cdk::Amount::from(1_u64)];
        assert_eq!(
            key.public_key.to_hex(),
            "03287106d3d2f1df660f7c7764e39e98051bca0c95feb9604336e9744de88eac68"
        );
        // m/129372'/129534'/0'/927402239'/5'
        let key = &keyset.keys[&cdk::Amount::from(32_u64)];
        assert_eq!(
            key.public_key.to_hex(),
            "03c5b66986d15100d1c0b342e012da7a954c7040c13d514ebc3b282ffa3a54651f"
        );
    }

    #[tokio::test]
    async fn test_swaprepository_info_debit_key() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let mut debit_repo = keys_test::MockRepository::new();

        let kid = keys_test::generate_random_keysetid();
        let info = cdk::mint::MintKeySetInfo {
            active: true,
            derivation_path: Default::default(),
            derivation_path_index: Default::default(),
            id: kid.into(),
            input_fee_ppk: Default::default(),
            max_order: Default::default(),
            unit: Default::default(),
            valid_from: Default::default(),
            valid_to: Default::default(),
        };

        quote_repo
            .expect_info()
            .with(eq(kid))
            .returning(|_| Ok(None));
        maturing_repo
            .expect_info()
            .with(eq(kid))
            .returning(|_| Ok(None));
        let cinfo = info.clone();
        debit_repo
            .expect_info()
            .with(eq(kid))
            .returning(move |_| Ok(Some(cinfo.clone())));

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.info(&kid).await.unwrap();
        assert_eq!(result, Some(info));
    }

    #[tokio::test]
    async fn test_swaprepository_info_maturing_key() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let debit_repo = keys_test::MockRepository::new();

        let kid = keys_test::generate_random_keysetid();
        let info = cdk::mint::MintKeySetInfo {
            active: true,
            derivation_path: Default::default(),
            derivation_path_index: Default::default(),
            id: kid.into(),
            input_fee_ppk: Default::default(),
            max_order: Default::default(),
            unit: Default::default(),
            valid_from: Default::default(),
            valid_to: Default::default(),
        };

        quote_repo
            .expect_info()
            .with(eq(kid))
            .returning(|_| Ok(None));
        let cinfo = info.clone();
        maturing_repo
            .expect_info()
            .with(eq(kid))
            .returning(move |_| Ok(Some(cinfo.clone())));

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.info(&kid).await.unwrap();
        assert_eq!(result, Some(info));
    }

    #[tokio::test]
    async fn test_swaprepository_info_quote_key() {
        let mut quote_repo = keys_test::MockRepository::new();
        let maturing_repo = keys_test::MockRepository::new();
        let debit_repo = keys_test::MockRepository::new();

        let kid = keys_test::generate_random_keysetid();
        let info = cdk::mint::MintKeySetInfo {
            active: true,
            derivation_path: Default::default(),
            derivation_path_index: Default::default(),
            id: kid.into(),
            input_fee_ppk: Default::default(),
            max_order: Default::default(),
            unit: Default::default(),
            valid_from: Default::default(),
            valid_to: Default::default(),
        };

        let cinfo = info.clone();
        quote_repo
            .expect_info()
            .with(eq(kid))
            .returning(move |_| Ok(Some(cinfo.clone())));

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.info(&kid).await.unwrap();
        assert_eq!(result, Some(info));
    }

    #[tokio::test]
    async fn test_swaprepository_keyset_debit_key() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let mut debit_repo = keys_test::MockRepository::new();

        let kid = keys_test::generate_random_keysetid();
        let set = cdk02::MintKeySet {
            id: kid.into(),
            keys: cdk01::MintKeys::new(Default::default()),
            unit: Default::default(),
        };

        quote_repo
            .expect_keyset()
            .with(eq(kid))
            .returning(|_| Ok(None));
        maturing_repo
            .expect_keyset()
            .with(eq(kid))
            .returning(|_| Ok(None));
        let cset = set.clone();
        debit_repo
            .expect_keyset()
            .with(eq(kid))
            .returning(move |_| Ok(Some(cset.clone())));

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.keyset(&kid).await.unwrap();
        assert_eq!(result, Some(set));
    }

    #[tokio::test]
    async fn test_swaprepository_keyset_maturing_key() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let debit_repo = keys_test::MockRepository::new();

        let kid = keys_test::generate_random_keysetid();
        let set = cdk02::MintKeySet {
            id: kid.into(),
            keys: cdk01::MintKeys::new(Default::default()),
            unit: Default::default(),
        };

        quote_repo
            .expect_keyset()
            .with(eq(kid))
            .returning(|_| Ok(None));
        let cset = set.clone();
        maturing_repo
            .expect_keyset()
            .with(eq(kid))
            .returning(move |_| Ok(Some(cset.clone())));

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.keyset(&kid).await.unwrap();
        assert_eq!(result, Some(set));
    }

    #[tokio::test]
    async fn test_swaprepository_keyset_quote_key() {
        let mut quote_repo = keys_test::MockRepository::new();
        let maturing_repo = keys_test::MockRepository::new();
        let debit_repo = keys_test::MockRepository::new();

        let kid = keys_test::generate_random_keysetid();
        let set = cdk02::MintKeySet {
            id: kid.into(),
            keys: cdk01::MintKeys::new(Default::default()),
            unit: Default::default(),
        };

        let cset = set.clone();
        quote_repo
            .expect_keyset()
            .with(eq(kid))
            .returning(move |_| Ok(Some(cset.clone())));

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.keyset(&kid).await.unwrap();
        assert_eq!(result, Some(set));
    }

    #[tokio::test]
    async fn test_swaprepository_replacing_keys_debit() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let mut debit_repo = keys_test::MockRepository::new();

        let in_kid = keys_test::generate_random_keysetid();
        let out_kid = keys_test::generate_random_keysetid();

        quote_repo
            .expect_info()
            .with(eq(in_kid))
            .returning(|_| Ok(None));
        maturing_repo
            .expect_info()
            .with(eq(in_kid))
            .returning(|_| Ok(None));
        debit_repo.expect_info_active().returning(move || {
            Ok(Some(cdk::mint::MintKeySetInfo {
                active: true,
                derivation_path: Default::default(),
                derivation_path_index: Default::default(),
                id: out_kid.into(),
                input_fee_ppk: Default::default(),
                max_order: Default::default(),
                unit: Default::default(),
                valid_from: Default::default(),
                valid_to: Default::default(),
            }))
        });

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.replacing_id(&in_kid).await.unwrap();
        assert_eq!(result, Some(out_kid));
    }

    #[tokio::test]
    async fn test_swaprepository_replacing_keys_maturing_active() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let debit_repo = keys_test::MockRepository::new();

        let kid = keys_test::generate_random_keysetid();

        quote_repo
            .expect_info()
            .with(eq(kid))
            .returning(|_| Ok(None));
        maturing_repo
            .expect_info()
            .with(eq(kid))
            .returning(move |_| {
                Ok(Some(cdk::mint::MintKeySetInfo {
                    active: true,
                    derivation_path: Default::default(),
                    derivation_path_index: Default::default(),
                    id: kid.into(),
                    input_fee_ppk: Default::default(),
                    max_order: Default::default(),
                    unit: Default::default(),
                    valid_from: Default::default(),
                    valid_to: Default::default(),
                }))
            });

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.replacing_id(&kid).await.unwrap();
        assert_eq!(result, Some(kid));
    }

    #[tokio::test]
    async fn test_swaprepository_replacing_keys_maturing_inactive() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let debit_repo = keys_test::MockRepository::new();

        let in_kid = keys_test::generate_random_keysetid();
        let maturity_date =
            chrono::NaiveDateTime::parse_from_str("2026-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")
                .unwrap()
                .and_utc();

        quote_repo
            .expect_info()
            .with(eq(in_kid))
            .returning(|_| Ok(None));
        maturing_repo
            .expect_info()
            .with(eq(in_kid))
            .returning(move |_| {
                Ok(Some(cdk::mint::MintKeySetInfo {
                    active: false,
                    derivation_path: Default::default(),
                    derivation_path_index: Some(0),
                    id: in_kid.into(),
                    input_fee_ppk: Default::default(),
                    max_order: Default::default(),
                    unit: Default::default(),
                    valid_from: Default::default(),
                    valid_to: Some(maturity_date.timestamp() as u64),
                }))
            });
        let maturity_kid = keys::generate_keyset_id_from_date(maturity_date, 1);
        maturing_repo
            .expect_info()
            .with(eq(maturity_kid))
            .returning(move |_| {
                Ok(Some(cdk::mint::MintKeySetInfo {
                    active: true,
                    derivation_path: Default::default(),
                    derivation_path_index: Some(1),
                    id: maturity_kid.into(),
                    input_fee_ppk: Default::default(),
                    max_order: Default::default(),
                    unit: Default::default(),
                    valid_from: Default::default(),
                    valid_to: Some(maturity_date.timestamp() as u64),
                }))
            });

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.replacing_id(&in_kid).await.unwrap();
        assert_eq!(result, Some(maturity_kid));
    }

    #[tokio::test]
    async fn test_swaprepository_replacing_keys_maturing_inactive_to_debit() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let mut debit_repo = keys_test::MockRepository::new();

        let in_kid = keys_test::generate_random_keysetid();
        let maturity_date =
            chrono::NaiveDateTime::parse_from_str("2026-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")
                .unwrap()
                .and_utc();

        quote_repo
            .expect_info()
            .with(eq(in_kid))
            .returning(|_| Ok(None));
        maturing_repo
            .expect_info()
            .with(eq(in_kid))
            .returning(move |_| {
                Ok(Some(cdk::mint::MintKeySetInfo {
                    active: false,
                    derivation_path: Default::default(),
                    derivation_path_index: Some(0),
                    id: in_kid.into(),
                    input_fee_ppk: Default::default(),
                    max_order: Default::default(),
                    unit: Default::default(),
                    valid_from: Default::default(),
                    valid_to: Some(maturity_date.timestamp() as u64),
                }))
            });
        let maturity_kid = keys::generate_keyset_id_from_date(maturity_date, 1);
        maturing_repo
            .expect_info()
            .with(eq(maturity_kid))
            .returning(move |_| Ok(None));
        let debit_kid = keys_test::generate_random_keysetid();
        debit_repo.expect_info_active().returning(move || {
            Ok(Some(cdk::mint::MintKeySetInfo {
                active: false,
                derivation_path: Default::default(),
                derivation_path_index: Some(0),
                id: debit_kid.into(),
                input_fee_ppk: Default::default(),
                max_order: Default::default(),
                unit: Default::default(),
                valid_from: Default::default(),
                valid_to: Some(maturity_date.timestamp() as u64),
            }))
        });

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.replacing_id(&in_kid).await.unwrap();
        assert_eq!(result, Some(debit_kid));
    }

    #[tokio::test]
    async fn test_swaprepository_replacing_keys_quote_to_maturing() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let debit_repo = keys_test::MockRepository::new();

        let in_kid = keys_test::generate_random_keysetid();
        let maturity_date =
            chrono::NaiveDateTime::parse_from_str("2026-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")
                .unwrap()
                .and_utc();

        quote_repo
            .expect_info()
            .with(eq(in_kid))
            .returning(move |_| {
                Ok(Some(cdk::mint::MintKeySetInfo {
                    active: false,
                    derivation_path: Default::default(),
                    derivation_path_index: Some(0),
                    id: in_kid.into(),
                    input_fee_ppk: Default::default(),
                    max_order: Default::default(),
                    unit: Default::default(),
                    valid_from: Default::default(),
                    valid_to: Some(maturity_date.timestamp() as u64),
                }))
            });
        let maturity_kid = keys::generate_keyset_id_from_date(maturity_date, 0);
        maturing_repo
            .expect_info()
            .with(eq(maturity_kid))
            .returning(move |_| {
                Ok(Some(cdk::mint::MintKeySetInfo {
                    active: true,
                    derivation_path: Default::default(),
                    derivation_path_index: Some(0),
                    id: maturity_kid.into(),
                    input_fee_ppk: Default::default(),
                    max_order: Default::default(),
                    unit: Default::default(),
                    valid_from: Default::default(),
                    valid_to: None,
                }))
            });

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.replacing_id(&in_kid).await.unwrap();
        assert_eq!(result, Some(maturity_kid));
    }
}
