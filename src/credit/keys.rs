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

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid keysetID {0}")]
    InvalidKeysetID(KeysetID),
    #[error("keyset with id {0} and path {1} already exists")]
    KeysetAlreadyExists(KeysetID, btc32::DerivationPath),
    #[error("no key for amount {0}")]
    NoKeyForAmount(cdk::Amount),
    #[error("cdk::nut01 error {0}")]
    CdkNut01(#[from] cdk01::Error),
    #[error("cdk::dhke error {0}")]
    CdkDHKE(#[from] cdk::dhke::Error),
    #[error("repository error {0}")]
    RepoError(#[from] Box<dyn std::error::Error>),
}
/// rework of cdk02::Id as they do not export internal fields
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
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
            id: digest.as_byte_array()[0..Self::BYTELEN]
                .try_into()
                .expect("cdk::KeysetID BYTELEN == 7"),
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
            id: bb[1..].try_into().expect("cdk::KeysetID BYTELEN == 7"),
        }
    }
}

impl std::convert::From<KeysetID> for cdk02::Id {
    fn from(id: KeysetID) -> Self {
        Self::from_bytes(&id.as_bytes()).expect("cdk::KeysetID BYTELEN == 7")
    }
}

impl std::fmt::Display for KeysetID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.version, hex::encode(self.id))
    }
}

// ---------- Keys Repository for creation
#[cfg_attr(test, mockall::automock)]
pub trait CreateRepository: Send + Sync {
    fn info(&self, id: &KeysetID) -> Option<cdk::mint::MintKeySetInfo>;
    fn store(&self, keyset: cdk02::MintKeySet, info: cdk::mint::MintKeySetInfo) -> std::result::Result<(), Box<dyn std::error::Error>>;
}

// ---------- Keys Factory
#[derive(Clone)]
pub struct Factory<Keys> {
    ctx: bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>,
    xpriv: btc32::Xpriv,
    repo: Keys,
    unit: cdk00::CurrencyUnit,
}

impl<Keys> Factory<Keys> {
    pub const MAX_ORDER: u8 = 20;
    pub const CURRENCY_UNIT: &'static str = "crsat";

    pub fn new(seed: &[u8], repo: Keys) -> Self {
        Self {
            ctx: bitcoin::secp256k1::Secp256k1::new(),
            xpriv: btc32::Xpriv::new_master(bitcoin::Network::Bitcoin, seed).expect("bitcoin FAIL"),
            repo,
            unit: cdk00::CurrencyUnit::Custom(String::from(Self::CURRENCY_UNIT)),
        }
    }
}

impl<Keys: CreateRepository> Factory<Keys> {
    // inspired by cdk::nut13, we attempt to generate keysets following a deterministic path
    // m/129372'/129534'/<keysetID>'/<quoteID>'/<rotateID>'/<amount_idx>'
    // 129372 is utf-8 for 🥜
    // 129534 is utf-8 for 🧾
    // <keysetID> is u32 from first 4bytes of sha256(keysetID)
    // <quoteID> is u32 from first 4bytes of sha256(quoteID)
    // <rotateID> is the rotating index, when newly generated index is 0
    pub fn generate(&self, keysetid: KeysetID, quote: uuid::Uuid) -> Result<cdk02::MintKeySet> {
        const MAX_INDEX: u32 = 2_u32.pow(31) - 1;
        let keyset_as_u = std::cmp::min(
            u32::from_be_bytes(
                Sha256::hash(&keysetid.as_bytes())[0..4]
                    .try_into()
                    .expect("a u32 is 4 bytes"),
            ),
            MAX_INDEX,
        );
        let quote_as_u = std::cmp::min(
            u32::from_be_bytes(
                Sha256::hash(quote.as_bytes())[0..4]
                    .try_into()
                    .expect("a u32 is 4 bytes"),
            ),
            MAX_INDEX,
        );
        let path = [
            btc32::ChildNumber::from_hardened_idx(129372).expect("129372 is a valid index"),
            btc32::ChildNumber::from_hardened_idx(129534).expect("129534 is a valid index"),
            btc32::ChildNumber::from_hardened_idx(keyset_as_u).expect("keyset is a valid index"),
            btc32::ChildNumber::from_hardened_idx(quote_as_u).expect("quote is a valid index"),
        ];
        let path = btc32::DerivationPath::from(path.as_slice());
        let indexed_path =
            path.child(btc32::ChildNumber::from_hardened_idx(0).expect("0 is a valid index"));
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
        let set = cdk02::MintKeySet {
            id: keysetid.into(),
            keys,
            unit: self.unit.clone(),
        };
        self.repo.store(set.clone(), info)?;

        Ok(set)
    }
}

pub fn sign_with_keys(
    keyset: &cdk02::MintKeySet,
    blind: &cdk00::BlindedMessage,
) -> Result<cdk00::BlindSignature> {
    if blind.keyset_id != keyset.id {
        return Err(Error::InvalidKeysetID(KeysetID::from(blind.keyset_id)));
    }
    let key = keyset
        .keys
        .get(&blind.amount)
        .ok_or(Error::NoKeyForAmount(blind.amount))?;
    let raw_signature = cdk::dhke::sign_message(&key.secret_key, &blind.blinded_secret)?;
    let signature = cdk00::BlindSignature {
        amount: blind.amount,
        c: raw_signature,
        keyset_id: keyset.id,
        dleq: None,
    };
    Ok(signature)
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

        let mut repo = MockCreateRepository::new();
        repo.expect_info().returning(|_| None);
        repo.expect_store().returning(|_, _| Ok(()));

        let factory = Factory::new(&seed, repo);

        let keyset = factory.generate(keyid, quote).unwrap();
        // m/129372'/129534'/2147383647'/927402239'/0'/0'
        let key = &keyset.keys[&cdk::Amount::from(1_u64)];
        assert_eq!(
            key.public_key.to_hex(),
            "02cc7583bba21bae84d15777a90a054ccf88056bb74b01d8440bc67dbdcccb5f85"
        );
        // m/129372'/129534'/2147383647'/927402239'/0'/5'
        let key = &keyset.keys[&cdk::Amount::from(32_u64)];
        assert_eq!(
            key.public_key.to_hex(),
            "02a2e66c769bc4b9615873fba6b4b22f45ea3a98ce63cd804ea94aebf4dfac7609"
        );
    }
}
