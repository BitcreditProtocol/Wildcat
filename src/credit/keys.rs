// ----- standard library imports
// ----- extra library imports
use bitcoin::bip32 as btc32;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;
use cdk::nuts::nut00 as cdk00;
use cdk::nuts::nut01 as cdk01;
use cdk::nuts::nut02 as cdk02;
use thiserror::Error;
// ----- local modules
// ----- local imports
use super::quotes;
use crate::keys::{generate_path_index_from_keysetid, KeysetID};
use crate::TStamp;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("keyset with id {0} and path {1} already exists")]
    KeysetAlreadyExists(KeysetID, btc32::DerivationPath),
    #[error("cdk::nut01 error {0}")]
    CdkNut01(#[from] cdk01::Error),
    #[error("repository error {0}")]
    Repository(#[from] Box<dyn std::error::Error>),
}

pub fn generate_keyset_id_from_bill(bill: &str, node: &str) -> KeysetID {
    let input = format!("{}{}", bill, node);
    let digest = Sha256::hash(input.as_bytes());
    KeysetID {
        version: cdk02::KeySetVersion::Version00,
        id: digest.as_byte_array()[0..KeysetID::BYTELEN]
            .try_into()
            .expect("cdk::KeysetID BYTELEN == 7"),
    }
}

// inspired by cdk::nut13, we attempt to generate keysets following a deterministic path
// m/129372'/129534'/<keysetID>'/<quoteID>'/<rotateID>'/<amount_idx>'
// 129372 is utf-8 for 🥜
// 129534 is utf-8 for 🧾
// <keysetID_idx> check generate_path_index_from_keysetid
// <quoteID_idx> check generate_path_idx_from_quoteid
fn generate_quote_keyset_path(kid: KeysetID, quote: uuid::Uuid) -> btc32::DerivationPath {
    let keyset_child = generate_path_index_from_keysetid(kid);
    let quote_child = quotes::generate_path_idx_from_quoteid(quote);
    let path = [
        btc32::ChildNumber::from_hardened_idx(129372).expect("129372 is a valid index"),
        btc32::ChildNumber::from_hardened_idx(129534).expect("129534 is a valid index"),
        keyset_child,
        quote_child,
    ];
    btc32::DerivationPath::from(path.as_slice())
}

fn generate_keyset_id_from_maturity_date(maturity_date: TStamp) -> KeysetID {
    let idx = (maturity_date - chrono::DateTime::UNIX_EPOCH).num_days() as u32;
    let mut kid = KeysetID {
        version: cdk02::KeySetVersion::Version00,
        id: Default::default(),
    };
    kid.id[0..4].copy_from_slice(&idx.to_be_bytes()[0..4]);
    kid
}

// inspired by cdk::nut13, we attempt to generate keysets following a deterministic path
// m/129372'/129534'/<keysetID>'/<quoteID>'/<rotateID>'/<amount_idx>'
// 129372 is utf-8 for 🥜
// 129534 is utf-8 for 🧾
// <maturity_idx> days from unix epoch
fn generate_maturing_keyset_path(maturity_date: TStamp) -> btc32::DerivationPath {
    let idx = (maturity_date - chrono::DateTime::UNIX_EPOCH).num_days() as u32;
    let maturity_child =
        btc32::ChildNumber::from_hardened_idx(idx).expect("maturity date is a valid index");
    let path = [
        btc32::ChildNumber::from_hardened_idx(129372).expect("129372 is a valid index"),
        btc32::ChildNumber::from_hardened_idx(129534).expect("129534 is a valid index"),
        maturity_child,
    ];
    btc32::DerivationPath::from(path.as_slice())
}

// ---------- Keys Repository for creation
#[cfg_attr(test, mockall::automock)]
pub trait CreateRepository: Send + Sync {
    fn info(&self, id: &KeysetID) -> Option<cdk::mint::MintKeySetInfo>;
    fn store(
        &self,
        keyset: cdk02::MintKeySet,
        info: cdk::mint::MintKeySetInfo,
    ) -> std::result::Result<(), Box<dyn std::error::Error>>;
}

// ---------- Keys Factory
#[derive(Clone)]
pub struct Factory<Keys> {
    ctx: bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>,
    xpriv: btc32::Xpriv,
    quote_keys: Keys,
    maturing_keys: Keys,
    unit: cdk00::CurrencyUnit,
}

impl<Keys> Factory<Keys> {
    pub const MAX_ORDER: u8 = 20;
    pub const CURRENCY_UNIT: &'static str = "crsat";

    pub fn new(seed: &[u8], quote_keys: Keys, maturing_keys: Keys) -> Self {
        Self {
            ctx: bitcoin::secp256k1::Secp256k1::new(),
            xpriv: btc32::Xpriv::new_master(bitcoin::Network::Bitcoin, seed).expect("bitcoin FAIL"),
            quote_keys,
            maturing_keys,
            unit: cdk00::CurrencyUnit::Custom(String::from(Self::CURRENCY_UNIT)),
        }
    }
}

impl<Keys: CreateRepository> Factory<Keys> {
    pub fn generate(
        &self,
        keysetid: KeysetID,
        quote: uuid::Uuid,
        bill_maturity_date: TStamp,
    ) -> Result<cdk02::MintKeySet> {
        let path = generate_quote_keyset_path(keysetid, quote);
        let info = self.quote_keys.info(&keysetid);
        if let Some(info) = info {
            if info.derivation_path == path {
                return Err(Error::KeysetAlreadyExists(keysetid, path));
            }
        }
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
        self.quote_keys.store(set.clone(), info)?;

        let kid = generate_keyset_id_from_maturity_date(bill_maturity_date);
        if self.maturing_keys.info(&kid).is_some() {
            return Ok(set);
        }

        let path = generate_maturing_keyset_path(bill_maturity_date);
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
            active: false,
            valid_from: chrono::Utc::now().timestamp() as u64,
            valid_to: Some(bill_maturity_date.timestamp() as u64),
            derivation_path: path,
            derivation_path_index: Some(0),
            max_order: Self::MAX_ORDER,
            input_fee_ppk: 0,
        };
        self.maturing_keys.store(keyset.clone(), info)?;

        Ok(set)
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_keys_factory_generate() {
        let seed = bip39::Mnemonic::from_str("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").unwrap().to_seed("");

        let keyid = KeysetID::from(cdk02::Id::from_bytes(&[0u8; 8]).unwrap());
        let quote = uuid::Uuid::from_u128(0);
        let maturity = chrono::DateTime::parse_from_rfc3339("2021-01-01T00:00:00Z")
            .unwrap()
            .to_utc();

        let mut maturingkeys_repo = MockCreateRepository::new();
        maturingkeys_repo.expect_info().returning(|_| None);
        maturingkeys_repo.expect_store().returning(|_, _| Ok(()));
        let mut quotekeys_repo = MockCreateRepository::new();
        quotekeys_repo.expect_info().returning(|_| None);
        quotekeys_repo.expect_store().returning(|_, _| Ok(()));

        let factory = Factory::new(&seed, quotekeys_repo, maturingkeys_repo);

        let keyset = factory.generate(keyid, quote, maturity).unwrap();
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
}
