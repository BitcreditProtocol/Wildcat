// ----- standard library imports
// ----- extra library imports
use bitcoin::{
    bip32 as btc32,
    hashes::{sha256::Hash as Sha256, Hash},
};
use cashu::{
    dhke as cdk_dhke, nut00 as cdk00, nut02 as cdk02, nut10 as cdk10, nut11 as cdk11,
    nut12 as cdk12, nut14 as cdk14, Amount as cdk_Amount,
};
use cdk_common::mint as cdk_mint;
use thiserror::Error;
use uuid::Uuid;
// ----- local imports

// ----- end imports

pub type KeysetEntry = (cdk_mint::MintKeySetInfo, cdk02::MintKeySet);

pub fn extend_path_from_uuid(id: Uuid, parent: &btc32::DerivationPath) -> btc32::DerivationPath {
    const MAX_INDEX: u32 = 2_u32.pow(31) - 1;
    let (half_1, half_2) = id.as_u64_pair();
    let half_1 = half_1.to_be_bytes();
    let half_2 = half_2.to_be_bytes();
    let b_1: [u8; 4] = std::convert::TryFrom::try_from(&half_1[0..4]).expect("half_1/1 is 4 bytes");
    let b_2: [u8; 4] = std::convert::TryFrom::try_from(&half_1[4..]).expect("half_1/2 is 4 bytes");
    let b_3: [u8; 4] = std::convert::TryFrom::try_from(&half_2[0..4]).expect("half_2/1 is 4 bytes");
    let b_4: [u8; 4] = std::convert::TryFrom::try_from(&half_2[4..]).expect("half_2/2 is 4 bytes");

    let child_1 = u32::from_be_bytes(b_1);
    let child_1 = std::cmp::min(MAX_INDEX, child_1);
    let child_2 = u32::from_be_bytes(b_2);
    let child_2 = std::cmp::min(MAX_INDEX, child_2);
    let child_3 = u32::from_be_bytes(b_3);
    let child_3 = std::cmp::min(MAX_INDEX, child_3);
    let child_4 = u32::from_be_bytes(b_4);
    let child_4 = std::cmp::min(MAX_INDEX, child_4);

    let relative_path = [
        btc32::ChildNumber::from_normal_idx(child_1).expect("child_1 is a valid index"),
        btc32::ChildNumber::from_normal_idx(child_2).expect("child_2 is a valid index"),
        btc32::ChildNumber::from_normal_idx(child_3).expect("child_3 is a valid index"),
        btc32::ChildNumber::from_normal_idx(child_4).expect("child_4 is a valid index"),
    ];
    parent.extend(relative_path)
}

pub type SignWithKeysResult<T> = std::result::Result<T, SignWithKeysError>;
#[derive(Debug, Error)]
pub enum SignWithKeysError {
    #[error("no key for amount {0}")]
    NoKeyForAmount(cdk_Amount),
    #[error("cdk::dhke error {0}")]
    CdkDHKE(#[from] cdk_dhke::Error),
    #[error("cdk::nut12 error {0}")]
    CdkNut12(#[from] cdk12::Error),
}
pub fn sign_with_keys(
    keyset: &cdk02::MintKeySet,
    blind: &cdk00::BlindedMessage,
) -> SignWithKeysResult<cdk00::BlindSignature> {
    let key = keyset
        .keys
        .get(&blind.amount)
        .ok_or(SignWithKeysError::NoKeyForAmount(blind.amount))?;
    let raw_signature = cdk_dhke::sign_message(&key.secret_key, &blind.blinded_secret)?;
    let mut signature = cdk00::BlindSignature {
        amount: blind.amount,
        c: raw_signature,
        keyset_id: keyset.id,
        dleq: None,
    };
    signature.add_dleq_proof(&blind.blinded_secret, &key.secret_key)?;
    Ok(signature)
}

pub type VerifyWithKeysResult<T> = std::result::Result<T, VerifyWithKeysError>;
#[derive(Debug, Error)]
pub enum VerifyWithKeysError {
    #[error("no key for amount {0}")]
    NoKeyForAmount(cdk_Amount),
    #[error("cdk::dhke error {0}")]
    CdkDHKE(#[from] cdk_dhke::Error),
    #[error("Nut11 error {0}")]
    Cdk11(#[from] cdk11::Error),
    #[error("Nut14 error {0}")]
    Cdk14(#[from] cdk14::Error),
}
pub fn verify_with_keys(
    keyset: &cdk02::MintKeySet,
    proof: &cdk00::Proof,
) -> VerifyWithKeysResult<()> {
    // ref: https://docs.rs/cdk/latest/cdk/mint/struct.Mint.html#method.verify_proof
    if let Ok(secret) = <&cashu::secret::Secret as TryInto<cdk10::Secret>>::try_into(&proof.secret)
    {
        match secret.kind() {
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
        .ok_or(VerifyWithKeysError::NoKeyForAmount(proof.amount))?;
    cashu::dhke::verify_message(&keypair.secret_key, proof.c, proof.secret.as_bytes())?;
    Ok(())
}

pub type SchnorrSignBorshResult<T> = std::result::Result<T, SchnorrBorshMsgError>;
#[derive(Debug, Error)]
pub enum SchnorrBorshMsgError {
    #[error("Borsh error {0}")]
    Borsh(borsh::io::Error),
    #[error("Secp256k1 error {0}")]
    Secp256k1(bitcoin::secp256k1::Error),
}

pub fn schnorr_sign_borsh_msg_with_key<Message>(
    msg: &Message,
    keys: &bitcoin::secp256k1::Keypair,
) -> SchnorrSignBorshResult<bitcoin::secp256k1::schnorr::Signature>
where
    Message: borsh::BorshSerialize,
{
    let serialized = borsh::to_vec(&msg).map_err(SchnorrBorshMsgError::Borsh)?;
    let sha = Sha256::hash(&serialized);
    let secp_msg = bitcoin::secp256k1::Message::from_digest(*sha.as_ref());

    Ok(bitcoin::secp256k1::global::SECP256K1.sign_schnorr(&secp_msg, keys))
}

pub fn schnorr_verify_borsh_msg_with_key<Message>(
    msg: &Message,
    signature: &bitcoin::secp256k1::schnorr::Signature,
    key: &bitcoin::secp256k1::XOnlyPublicKey,
) -> SchnorrSignBorshResult<()>
where
    Message: borsh::BorshSerialize,
{
    let serialized = borsh::to_vec(&msg).map_err(SchnorrBorshMsgError::Borsh)?;
    let sha = Sha256::hash(&serialized);
    let secp_msg = bitcoin::secp256k1::Message::from_digest(*sha.as_ref());

    bitcoin::secp256k1::global::SECP256K1
        .verify_schnorr(signature, &secp_msg, key)
        .map_err(SchnorrBorshMsgError::Secp256k1)
}

#[cfg(any(feature = "test-utils", test))]
pub mod test_utils {

    use super::*;
    use bitcoin::bip32::DerivationPath;
    use cashu::nuts::nut01 as cdk01;
    use cashu::secret as cdk_secret;
    use cdk_common::mint as cdk_mint;
    use rand::RngCore;

    pub fn generate_random_keypair() -> bitcoin::secp256k1::Keypair {
        let mut rng = rand::thread_rng();
        bitcoin::secp256k1::Keypair::new(bitcoin::secp256k1::global::SECP256K1, &mut rng)
    }

    pub fn generate_random_keysetid() -> cdk02::Id {
        const ID_BYTE_LEN: usize = 7;
        let mut id_bytes = [0u8; ID_BYTE_LEN + 1];
        id_bytes[0] = cdk02::KeySetVersion::Version00 as u8;
        rand::thread_rng().fill_bytes(&mut id_bytes[1..]);
        cdk02::Id::from_bytes(&id_bytes).expect("Keyset ID is valid")
    }

    pub fn generate_keyset() -> (cdk_mint::MintKeySetInfo, cdk02::MintKeySet) {
        let path = DerivationPath::master();
        let set = cdk02::MintKeySet::generate_from_seed(
            secp256k1::global::SECP256K1,
            &[],
            10,
            cdk00::CurrencyUnit::Sat,
            path.clone(),
            None,
            cdk02::KeySetVersion::Version00,
        );
        let info = cdk_mint::MintKeySetInfo {
            id: set.id,
            active: true,
            unit: cdk00::CurrencyUnit::Sat,
            valid_from: 0,
            final_expiry: None,
            derivation_path_index: None,
            derivation_path: path,
            input_fee_ppk: 0,
            max_order: 10,
        };
        (info, set)
    }
    pub fn generate_random_keyset() -> (cdk_mint::MintKeySetInfo, cdk02::MintKeySet) {
        let path = DerivationPath::master();
        let mut random_seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut random_seed);
        let set = cdk02::MintKeySet::generate_from_seed(
            secp256k1::global::SECP256K1,
            &random_seed,
            10,
            cdk00::CurrencyUnit::Sat,
            path.clone(),
            None,
            cdk02::KeySetVersion::Version00,
        );
        let info = cdk_mint::MintKeySetInfo {
            id: set.id,
            active: true,
            unit: cdk00::CurrencyUnit::Sat,
            valid_from: 0,
            final_expiry: None,
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

        verify_with_keys(&keyset, &p).unwrap();
    }
}
