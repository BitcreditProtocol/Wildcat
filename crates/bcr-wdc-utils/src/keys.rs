// ----- standard library imports
use std::collections::HashMap;
// ----- extra library imports
use bcr_common::{cashu, ecash};
use bitcoin::bip32 as btc32;
// ----- local imports

// ----- end imports

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintKeysEntry {
    pub id: cashu::Id,
    pub unit: cashu::CurrencyUnit,
    pub active: bool,
    pub valid_from: u64,
    pub derivation_path: btc32::DerivationPath,
    pub derivation_path_index: Option<u32>,
    pub amounts: Vec<u64>,
    pub input_fee_ppk: u64,
    pub final_expiry: Option<u64>,
    pub keys: cashu::nut01::MintKeys,
}

impl From<MintKeysEntry> for ecash::MintKeySet {
    fn from(entry: MintKeysEntry) -> Self {
        Self {
            id: entry.id,
            unit: entry.unit,
            input_fee_ppk: entry.input_fee_ppk,
            final_expiry: entry.final_expiry,
            keys: entry.keys,
        }
    }
}

impl From<MintKeysEntry> for ecash::MintKeySetInfo {
    fn from(entry: MintKeysEntry) -> Self {
        Self {
            id: entry.id,
            unit: entry.unit,
            active: entry.active,
            valid_from: entry.valid_from,
            derivation_path: entry.derivation_path,
            derivation_path_index: entry.derivation_path_index,
            amounts: entry.amounts,
            input_fee_ppk: entry.input_fee_ppk,
            final_expiry: entry.final_expiry,
        }
    }
}

pub fn from_entry(entry: MintKeysEntry) -> (ecash::MintKeySetInfo, ecash::MintKeySet) {
    let MintKeysEntry {
        id,
        unit,
        active,
        valid_from,
        derivation_path,
        derivation_path_index,
        amounts,
        input_fee_ppk,
        final_expiry,
        keys,
    } = entry;
    let info = ecash::MintKeySetInfo {
        id,
        unit: unit.clone(),
        active,
        valid_from,
        derivation_path,
        derivation_path_index,
        amounts,
        input_fee_ppk,
        final_expiry,
    };
    let keyset = ecash::MintKeySet {
        id,
        unit,
        input_fee_ppk,
        final_expiry,
        keys,
    };
    (info, keyset)
}

pub fn to_entry(info: ecash::MintKeySetInfo, keyset: ecash::MintKeySet) -> MintKeysEntry {
    MintKeysEntry {
        id: info.id,
        unit: info.unit,
        active: info.active,
        valid_from: info.valid_from,
        derivation_path: info.derivation_path,
        derivation_path_index: info.derivation_path_index,
        amounts: info.amounts,
        input_fee_ppk: info.input_fee_ppk,
        final_expiry: info.final_expiry,
        keys: keyset.keys,
    }
}

pub fn kinfos_list_to_map(
    kinfos: Vec<ecash::MintKeySetInfo>,
) -> HashMap<cashu::Id, ecash::KeySetInfo> {
    HashMap::from_iter(kinfos.into_iter().map(|kinfo| (kinfo.id, kinfo.into())))
}

pub use bcr_common::core::keys::{to_fee_and_amounts, to_keyset};

#[cfg(any(feature = "test-utils", test))]
pub mod test_utils {

    use super::*;
    use bcr_common::cashu::secret as cdk_secret;

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
