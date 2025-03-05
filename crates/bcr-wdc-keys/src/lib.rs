// ----- standard library imports
// ----- extra library imports
use bitcoin::bip32 as btc32;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;
use cashu::dhke as cdk_dhke;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut02 as cdk02;
use cashu::Amount as cdk_Amount;
use thiserror::Error;
use uuid::Uuid;
// ----- local modules
pub mod id;
#[cfg(feature = "persistence")]
pub mod persistence;
// ----- local imports
pub use crate::id::KeysetID;

type TStamp = chrono::DateTime<chrono::Utc>;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("no key for amount {0}")]
    NoKeyForAmount(cdk_Amount),
    #[error("cdk::dhke error {0}")]
    CdkDHKE(#[from] cdk_dhke::Error),
    #[error("invalid timestamp {0}")]
    TStamp(TStamp),
}

pub fn generate_path_index_from_keysetid(kid: KeysetID) -> btc32::ChildNumber {
    const MAX_INDEX: u32 = 2_u32.pow(31) - 1;
    let ukid = std::cmp::min(u32::from(cdk02::Id::from(kid)), MAX_INDEX);
    btc32::ChildNumber::from_hardened_idx(ukid).expect("keyset is a valid index")
}

pub fn generate_path_index_from_id(id: Uuid) -> btc32::ChildNumber {
    const MAX_INDEX: u32 = 2_u32.pow(31) - 1;
    let sha_qid = Sha256::hash(id.as_bytes());
    let u_qid = u32::from_be_bytes(sha_qid[0..4].try_into().expect("a u32 is 4 bytes"));
    let idx_qid = std::cmp::min(u_qid, MAX_INDEX);
    btc32::ChildNumber::from_hardened_idx(idx_qid).expect("keyset is a valid index")
}

// inspired by cdk::nut13, we attempt to generate keysets following a deterministic path
// m/129372'/129534'/<keysetID>'/<quoteID>'/<rotateID>'/<amount_idx>'
// 129372 is utf-8 for 🥜
// 129534 is utf-8 for 🧾
// <keysetID_idx> check generate_path_index_from_keysetid
// <Uuid> optional: check generate_path_idx_from_id
pub fn generate_keyset_path(kid: KeysetID, id: Option<uuid::Uuid>) -> btc32::DerivationPath {
    let keyset_child = generate_path_index_from_keysetid(kid);
    let mut path = vec![
        btc32::ChildNumber::from_hardened_idx(129372).expect("129372 is a valid index"),
        btc32::ChildNumber::from_hardened_idx(129534).expect("129534 is a valid index"),
        keyset_child,
    ];
    if let Some(id) = id {
        let quote_child = generate_path_index_from_id(id);
        path.push(quote_child);
    }
    btc32::DerivationPath::from(path.as_slice())
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

pub fn sign_with_keys(
    keyset: &cdk02::MintKeySet,
    blind: &cdk00::BlindedMessage,
) -> Result<cdk00::BlindSignature> {
    let key = keyset
        .keys
        .get(&blind.amount)
        .ok_or(Error::NoKeyForAmount(blind.amount))?;
    let raw_signature = cdk_dhke::sign_message(&key.secret_key, &blind.blinded_secret)?;
    let signature = cdk00::BlindSignature {
        amount: blind.amount,
        c: raw_signature,
        keyset_id: keyset.id,
        dleq: None,
    };
    Ok(signature)
}

#[cfg(feature = "test-utils")]
pub mod test_utils {

    use super::*;
    use bitcoin::bip32::DerivationPath;
    use once_cell::sync::Lazy;
    use std::str::FromStr;

    static SECPCTX: Lazy<bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>> =
        Lazy::new(bitcoin::secp256k1::Secp256k1::new);

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
