// ----- standard library imports
// ----- extra library imports
use bcr_common::{cashu, cdk_common::mint as cdk_mint};
// ----- local imports

// ----- end imports

pub type KeysetEntry = (cdk_mint::MintKeySetInfo, cashu::MintKeySet);

#[cfg(any(feature = "test-utils", test))]
pub mod test_utils {

    use super::*;
    use bcr_common::cashu::{self, nut02::KeySetVersion, secret as cdk_secret};
    use bitcoin::bip32::DerivationPath;
    use rand::RngCore;

    pub fn generate_random_keypair() -> bitcoin::secp256k1::Keypair {
        let mut rng = rand::thread_rng();
        bitcoin::secp256k1::Keypair::new(bitcoin::secp256k1::global::SECP256K1, &mut rng)
    }

    pub fn generate_random_keysetid() -> cashu::Id {
        const ID_BYTE_LEN: usize = 7;
        let mut id_bytes = [0u8; ID_BYTE_LEN + 1];
        id_bytes[0] = KeySetVersion::Version00 as u8;
        rand::thread_rng().fill_bytes(&mut id_bytes[1..]);
        cashu::Id::from_bytes(&id_bytes).expect("Keyset ID is valid")
    }

    pub fn generate_keyset() -> (cdk_mint::MintKeySetInfo, cashu::MintKeySet) {
        let path = DerivationPath::master();
        let denominations: Vec<u64> = (0..10).map(|i| 2u64.pow(i)).collect();
        let set = cashu::MintKeySet::generate_from_seed(
            secp256k1::global::SECP256K1,
            &[],
            &denominations,
            cashu::CurrencyUnit::Sat,
            path.clone(),
            None,
            KeySetVersion::Version00,
        );
        let info = cdk_mint::MintKeySetInfo {
            id: set.id,
            amounts: denominations,
            active: true,
            unit: cashu::CurrencyUnit::Sat,
            valid_from: 0,
            final_expiry: None,
            derivation_path_index: None,
            derivation_path: path,
            input_fee_ppk: 0,
            max_order: 10,
        };
        (info, set)
    }
    pub fn generate_random_keyset() -> (cdk_mint::MintKeySetInfo, cashu::MintKeySet) {
        let path = DerivationPath::master();
        let mut random_seed = [0u8; 32];
        let denominations: Vec<u64> = (0..10).map(|i| 2u64.pow(i)).collect();
        rand::thread_rng().fill_bytes(&mut random_seed);
        let set = cashu::MintKeySet::generate_from_seed(
            secp256k1::global::SECP256K1,
            &random_seed,
            &denominations,
            cashu::CurrencyUnit::Sat,
            path.clone(),
            None,
            KeySetVersion::Version00,
        );
        let info = cdk_mint::MintKeySetInfo {
            id: set.id,
            active: true,
            amounts: denominations,
            unit: cashu::CurrencyUnit::Sat,
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
        kid: cashu::Id,
        amount: cashu::Amount,
    ) -> (cashu::BlindedMessage, cdk_secret::Secret, cashu::SecretKey) {
        let secret = cdk_secret::Secret::new(rand::random::<u64>().to_string());
        let (b_, r) =
            cashu::dhke::blind_message(secret.as_bytes(), None).expect("cdk_dhke::blind_message");
        (cashu::BlindedMessage::new(amount, kid, b_), secret, r)
    }

    pub const RANDOMS: [&str; 6] = [
        "0244e4420934530b2bdf5161f4c88b3c4f923db158741da51f3bb22b579495862e",
        "03244bce3f2ea7b12acd2004a6c629acf9d01e7eceadfd7f4ce6f7a09134a84474",
        "0212612cddd9e1aa368c500654538c71ebdf70d5bc4a1b642f9c963269505514cc",
        "0292abc8e9eb2935f0ae6fcf7c491ea124a5860ed954e339a0b7f549cd8c190500",
        "02cc8e0448596f0aaec2c62ef02e5a36f53a4e8b7d5a9e906d2c1f8d5cd738ccae",
        "027a238c992c4a5ea59502b2d6b52e6466bf2a775191cbfaf29b9311e8352d99dc",
    ];

    pub fn publics() -> Vec<cashu::PublicKey> {
        RANDOMS
            .iter()
            .map(|key| cashu::PublicKey::from_hex(key).unwrap())
            .collect()
    }
}
