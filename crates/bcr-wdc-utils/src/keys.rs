// ----- standard library imports
use std::collections::HashMap;
// ----- extra library imports
use bcr_common::{cashu, ecash};
use bitcoin::bip32 as btc32;
// ----- local imports

// ----- end imports

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintKeysEntry {
    pub id: ecash::Id,
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
            id: entry.id.into(),
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
            id: entry.id.into(),
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
        id: id.into(),
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
        id: id.into(),
        unit,
        input_fee_ppk,
        final_expiry,
        keys,
    };
    (info, keyset)
}

pub fn to_entry(info: ecash::MintKeySetInfo, keyset: ecash::MintKeySet) -> MintKeysEntry {
    MintKeysEntry {
        id: info.id.into(),
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
    HashMap::from_iter(
        kinfos
            .into_iter()
            .map(|kinfo| (kinfo.id.into(), kinfo.into())),
    )
}

pub use bcr_common::core::keys::{to_fee_and_amounts, to_keyset};
