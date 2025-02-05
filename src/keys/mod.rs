// ----- standard library imports
// ----- extra library imports
use bitcoin::bip32 as btc32;
use cdk::nuts::nut00 as cdk00;
use cdk::nuts::nut02 as cdk02;
use thiserror::Error;
// ----- local modules
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("no key for amount {0}")]
    NoKeyForAmount(cdk::Amount),
    #[error("cdk::dhke error {0}")]
    CdkDHKE(#[from] cdk::dhke::Error),
}

pub type KeysetEntry = (cdk::mint::MintKeySetInfo, cdk02::MintKeySet);
/// rework of cdk02::Id as they do not export internal fields
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub struct KeysetID {
    pub version: cdk02::KeySetVersion,
    pub id: [u8; Self::BYTELEN],
}

impl KeysetID {
    pub const BYTELEN: usize = 7;

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
        cdk02::Id::from(*self).fmt(f)
    }
}

pub fn generate_path_index_from_keysetid(kid: KeysetID) -> btc32::ChildNumber {
    const MAX_INDEX: u32 = 2_u32.pow(31) - 1;
    let ukid = std::cmp::min(u32::from(cdk02::Id::from(kid)), MAX_INDEX);
    btc32::ChildNumber::from_hardened_idx(ukid).expect("keyset is a valid index")
}

pub fn sign_with_keys(
    keyset: &cdk02::MintKeySet,
    blind: &cdk00::BlindedMessage,
) -> Result<cdk00::BlindSignature> {
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
pub mod tests {
    use super::*;
    use bitcoin::bip32::DerivationPath;
    use cdk::nuts::nut00 as cdk00;
    use cdk::nuts::nut02 as cdk02;
    use once_cell::sync::Lazy;
    use std::str::FromStr;

    static SECPCTX: Lazy<bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>> =
        Lazy::new(|| bitcoin::secp256k1::Secp256k1::new());

    pub fn generate_random_keysetid() -> KeysetID {
        KeysetID {
            version: cdk02::KeySetVersion::Version00,
            id: rand::random(),
        }
    }

    pub fn generate_keyset() -> cdk02::MintKeySet {
        let path = DerivationPath::from_str("m/0'/0").unwrap();
        cdk02::MintKeySet::generate_from_seed(&SECPCTX, &[], 10, cdk00::CurrencyUnit::Sat, path)
    }
}
