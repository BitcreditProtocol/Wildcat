// ----- standard library imports
// ----- extra library imports
use bitcoin::bip32 as btc32;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;
use cashu::dhke as cdk_dhke;
use cashu::mint as cdk_mint;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut02 as cdk02;
use cashu::nuts::nut10 as cdk10;
use cashu::nuts::nut11 as cdk11;
use cashu::nuts::nut14 as cdk14;
use cashu::Amount as cdk_Amount;
use thiserror::Error;
use uuid::Uuid;
// ----- local modules
pub mod id;
#[cfg(feature = "persistence")]
pub mod persistence;
// ----- local imports
pub use crate::id::KeysetID;

pub type KeysetEntry = (cdk_mint::MintKeySetInfo, cdk02::MintKeySet);
type TStamp = chrono::DateTime<chrono::Utc>;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("no key for amount {0}")]
    NoKeyForAmount(cdk_Amount),
    #[error("cdk::dhke error {0}")]
    CdkDHKE(#[from] cdk_dhke::Error),
    #[error("Nut11 error {0}")]
    Cdk11(#[from] cdk11::Error),
    #[error("Nut14 error {0}")]
    Cdk14(#[from] cdk14::Error),
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

pub fn verify_with_keys(keyset: &cdk02::MintKeySet, proof: cdk00::Proof) -> Result<()> {
    // ref: https://docs.rs/cdk/latest/cdk/mint/struct.Mint.html#method.verify_proof
    if let Ok(secret) = <&cashu::secret::Secret as TryInto<cdk10::Secret>>::try_into(&proof.secret)
    {
        match secret.kind {
            cashu::nuts::Kind::P2PK => {
                proof.verify_p2pk()?;
            }
            cashu::nuts::Kind::HTLC => {
                proof.verify_htlc()?;
            }
        }
    }

    let keypair = keyset
        .keys
        .get(&proof.amount)
        .ok_or(Error::NoKeyForAmount(proof.amount))?;
    cashu::dhke::verify_message(&keypair.secret_key, proof.c, proof.secret.as_bytes())?;
    Ok(())
}

pub fn into_secp256k1_msg(msg: &[u8]) -> bitcoin::secp256k1::Message {
    let sha = Sha256::hash(msg);
    bitcoin::secp256k1::Message::from_digest(*sha.as_ref())
}

#[cfg(feature = "test-utils")]
pub mod test_utils {

    use super::*;
    use bitcoin::bip32::DerivationPath;
    use cashu::mint as cdk_mint;
    use cashu::nuts::nut01 as cdk01;
    use cashu::secret as cdk_secret;
    use once_cell::sync::Lazy;

    static SECPCTX: Lazy<bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>> =
        Lazy::new(bitcoin::secp256k1::Secp256k1::new);

    pub fn generate_random_keysetid() -> KeysetID {
        KeysetID {
            version: cdk02::KeySetVersion::Version00,
            id: rand::random(),
        }
    }

    pub fn generate_keyset() -> (cdk_mint::MintKeySetInfo, cdk02::MintKeySet) {
        let path = DerivationPath::master();
        let set = cdk02::MintKeySet::generate_from_seed(
            &SECPCTX,
            &[],
            10,
            cdk00::CurrencyUnit::Sat,
            path.clone(),
        );
        let info = cdk_mint::MintKeySetInfo {
            id: set.id,
            active: true,
            unit: cdk00::CurrencyUnit::Sat,
            valid_from: 0,
            valid_to: None,
            derivation_path_index: None,
            derivation_path: path,
            input_fee_ppk: 0,
            max_order: 10,
        };
        (info, set)
    }

    pub fn generate_blind(
        kid: cdk02::Id,
        amount: cdk_Amount,
    ) -> (cdk00::BlindedMessage, cdk_secret::Secret, cdk01::SecretKey) {
        let secret = cdk_secret::Secret::new(rand::random::<u64>().to_string());
        let (b_, r) =
            cdk_dhke::blind_message(secret.as_bytes(), None).expect("cdk_dhke::blind_message");
        (cdk00::BlindedMessage::new(amount, kid, b_), secret, r)
    }

    pub const RANDOMS: [&str; 6] = [
        "0244e4420934530b2bdf5161f4c88b3c4f923db158741da51f3bb22b579495862e",
        "03244bce3f2ea7b12acd2004a6c629acf9d01e7eceadfd7f4ce6f7a09134a84474",
        "0212612cddd9e1aa368c500654538c71ebdf70d5bc4a1b642f9c963269505514cc",
        "0292abc8e9eb2935f0ae6fcf7c491ea124a5860ed954e339a0b7f549cd8c190500",
        "02cc8e0448596f0aaec2c62ef02e5a36f53a4e8b7d5a9e906d2c1f8d5cd738ccae",
        "027a238c992c4a5ea59502b2d6b52e6466bf2a775191cbfaf29b9311e8352d99dc",
    ];

    pub fn publics() -> Vec<cdk01::PublicKey> {
        RANDOMS
            .iter()
            .map(|key| cdk01::PublicKey::from_hex(key).unwrap())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cashu::Amount;

    #[test]
    fn sign_verify_message() {
        let (_, keyset) = test_utils::generate_keyset();
        let (blind, secret, secretkey) = test_utils::generate_blind(keyset.id, Amount::from(8));
        let sig = sign_with_keys(&keyset, &blind).unwrap();
        let mintpub = keyset.keys.get(&blind.amount).unwrap().public_key;
        let c = cashu::dhke::unblind_message(&sig.c, &secretkey, &mintpub).unwrap();
        let p = cdk00::Proof::new(blind.amount, keyset.id, secret, c);

        verify_with_keys(&keyset, p).unwrap();
    }
}
