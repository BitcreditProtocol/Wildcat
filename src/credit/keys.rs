#![allow(dead_code)]
// ----- standard library imports
// ----- extra library imports
use bitcoin::bip32 as btc32;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;
use cdk::nuts::nut00 as cdk00;
use cdk::nuts::nut01 as cdk01;
use cdk::nuts::nut02 as cdk02;
// ----- local modules
// ----- local imports
use super::{Result, Error};

/// rework of cdk02::Id as they do not export internal fields
#[derive(Debug, Clone, Copy)]
pub struct KeysetID {
    pub version: cdk02::KeySetVersion,
    pub id: [u8; Self::BYTELEN],
}

impl KeysetID {
    pub const BYTELEN: usize = 7;

    pub fn new(bill: &str, endorser: &str) -> Self {
        let input = format!("{}{}", bill, endorser);
        let digest = Sha256::hash(input.as_bytes());
        Self {
            version: cdk02::KeySetVersion::Version00,
            id: digest.as_byte_array()[0..Self::BYTELEN].try_into().unwrap(),
        }
    }

    pub fn as_bytes(&self) -> [u8; Self::BYTELEN + 1] {
        let mut bytes = [0u8; Self::BYTELEN + 1];
        bytes[0] = self.version as u8;
        bytes[1..].copy_from_slice(&self.id);
        bytes
    }
}

impl std::cmp::PartialEq<cdk02::Id> for KeysetID {
    fn eq(&self, other: &cdk02::Id) -> bool {
        other.as_bytes() == self.as_bytes()
    }
}

impl std::convert::From<cdk02::Id> for KeysetID {
    fn from(id: cdk02::Id) -> Self {
        let bb = id.to_bytes();
        assert_eq!(bb.len(), Self::BYTELEN + 1);
        assert_eq!(bb[0], cdk02::KeySetVersion::Version00.to_byte());
        Self {
            version: cdk02::KeySetVersion::Version00,
            id: bb[1..].try_into().unwrap(),
        }
    }
}

impl std::convert::From<KeysetID> for cdk02::Id {
    fn from(id: KeysetID) -> Self {
        Self::from_bytes(&id.as_bytes()).unwrap()
    }
}

// ---------- KeysRepository
#[cfg_attr(test, mockall::automock)]
pub trait KeysRepository: Send + Sync {
    fn info(&self, id: &KeysetID) -> Option<cdk::mint::MintKeySetInfo>;
    fn store(
        &self,
        id: KeysetID,
        keyset: cdk01::MintKeys,
        info: cdk::mint::MintKeySetInfo,
    ) -> Result<()>;
}

// ---------- KeysFactory
pub struct KeysFactory {
    ctx: bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>,
    xpriv: btc32::Xpriv,
    repo: Box<dyn KeysRepository>,
    unit : cdk00::CurrencyUnit,
}

impl KeysFactory {
    pub const MAX_ORDER: u8 = 20;

    pub fn new(seed: &[u8], repo: Box<dyn KeysRepository>) -> Self {
        Self {
            ctx: bitcoin::secp256k1::Secp256k1::new(),
            xpriv: btc32::Xpriv::new_master(bitcoin::Network::Bitcoin, seed).unwrap(),
            repo,
            unit: cdk00::CurrencyUnit::Custom(String::from("bcr")),
        }
    }

    fn generate(&self, keysetid: KeysetID, quote: uuid::Uuid) -> Result<cdk01::MintKeys> {
        let keyset_as_u = u32::from_be_bytes(keysetid.as_bytes()[0..4].try_into().unwrap());
        let quote_as_u =
            u32::from_be_bytes(Sha256::hash(quote.as_bytes())[0..4].try_into().unwrap());
        let path = [
            btc32::ChildNumber::from_hardened_idx(129372).unwrap(),
            btc32::ChildNumber::from_hardened_idx(129534).unwrap(),
            btc32::ChildNumber::from_hardened_idx(keyset_as_u).unwrap(),
            btc32::ChildNumber::from_hardened_idx(quote_as_u).unwrap(),
        ];
        let path = btc32::DerivationPath::from(path.as_slice());
        let indexed_path = path.child(btc32::ChildNumber::from_hardened_idx(0).unwrap());
        let info = self.repo.info(&keysetid);
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
            indexed_path,
        )
        .keys;

        let info = cdk::mint::MintKeySetInfo {
            id: keysetid.into(),
            unit: self.unit.clone(),
            active: false,
            valid_from: chrono::Utc::now().timestamp() as u64,
            valid_to: None,
            derivation_path: path,
            derivation_path_index: Some(0),
            max_order: Self::MAX_ORDER,
            input_fee_ppk: 0,
        };
        self.repo.store(keysetid, keys.clone(), info)?;


        Ok(keys)
    }
}

#[cfg(test)]
mod test {

    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_keys_factory_generate() {
        let seed = bip39::Mnemonic::from_str("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").unwrap().to_seed("");

        let keyid = KeysetID::from(cdk02::Id::from_bytes(&[0u8; 8]).unwrap());
        let quote = uuid::Uuid::from_u128(0);

        let mut repo = Box::new(MockKeysRepository::new());
        repo.expect_info().returning(|_| None);
        repo.expect_store().returning(|_, _, _| Ok(()));

        let factory = KeysFactory::new(&seed, repo);

        let keyset = factory.generate(keyid, quote).unwrap();
        // m/129372'/129534'/0'/927402239'/0'/0'
        let key = &keyset[&cdk::Amount::from(1_u64)];
        assert_eq!(
            key.public_key.to_hex(),
            "0303efe17c61fb94d20bfc5166466cf17a8a9c6de3957f8e6ff3e4f1cdf90bb059"
        );
        // m/129372'/129534'/0'/927402239'/0'/5'
        let key = &keyset[&cdk::Amount::from(32_u64)];
        assert_eq!(
            key.public_key.to_hex(),
            "034d6acfd2c8a433fa8af2b716f39cf73156d126e56c98e9fba594eed50d414737"
        );
    }
}
