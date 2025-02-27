// ----- standard library imports
// ----- extra library imports
use anyhow::{Error as AnyError, Result as AnyResult};
use async_trait::async_trait;
use bcr_wdc_keys as keys;
use bcr_wdc_keys::persistence::KeysetEntry;
use bcr_wdc_keys::KeysetID;
use bitcoin::bip32 as btc32;
use cashu::mint as cdk_mint;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut01 as cdk01;
use cashu::nuts::nut02 as cdk02;
use thiserror::Error;
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::quotes::KeyFactory;
use crate::TStamp;

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
    async fn load(&self, kid: &KeysetID, qid: Uuid) -> AnyResult<Option<KeysetEntry>>;
    async fn store(
        &self,
        qid: Uuid,
        keyset: cdk02::MintKeySet,
        info: cdk_mint::MintKeySetInfo,
    ) -> AnyResult<()>;
}

// ---------- Keys Factory
#[derive(Clone)]
pub struct Factory<QuoteKeys> {
    ctx: bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>,
    xpriv: btc32::Xpriv,
    quote_keys: QuoteKeys,
    unit: cdk00::CurrencyUnit,
}

impl<QuoteKeys> Factory<QuoteKeys> {
    pub const MAX_ORDER: u8 = 20;
    pub const CURRENCY_UNIT: &'static str = "crsat";

    pub fn new(seed: &[u8], quote_keys: QuoteKeys) -> Self {
        Self {
            ctx: bitcoin::secp256k1::Secp256k1::new(),
            xpriv: btc32::Xpriv::new_master(bitcoin::Network::Bitcoin, seed).expect("bitcoin FAIL"),
            quote_keys,
            unit: cdk00::CurrencyUnit::Custom(String::from(Self::CURRENCY_UNIT)),
        }
    }
}

#[async_trait]
impl<QuoteKeys> KeyFactory for Factory<QuoteKeys>
where
    QuoteKeys: QuoteBasedRepository,
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

        let info = cdk_mint::MintKeySetInfo {
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

        Ok(set)
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use cashu::Amount as cdk_Amount;
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

        let mut quotekeys_repo = MockQuoteBasedRepository::new();
        quotekeys_repo
            .expect_store()
            .with(eq(quote), always(), always())
            .returning(|_, _, _| Ok(()));
        //quotekeys_repo.expect_store().returning(|_, _| Ok(()));

        let factory = Factory::new(&seed, quotekeys_repo);

        let keyset = factory.generate(keyid, quote, maturity).await.unwrap();
        // m/129372'/129534'/0'/927402239'/0'
        let key = &keyset.keys[&cdk_Amount::from(1_u64)];
        assert_eq!(
            key.public_key.to_hex(),
            "03287106d3d2f1df660f7c7764e39e98051bca0c95feb9604336e9744de88eac68"
        );
        // m/129372'/129534'/0'/927402239'/5'
        let key = &keyset.keys[&cdk_Amount::from(32_u64)];
        assert_eq!(
            key.public_key.to_hex(),
            "03c5b66986d15100d1c0b342e012da7a954c7040c13d514ebc3b282ffa3a54651f"
        );
    }
}
