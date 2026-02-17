// ----- standard library imports
// ----- extra library imports
use bcr_common::cashu::{self, nut02::KeySetVersion};
use bcr_common::cdk_common::mint::MintKeySetInfo;
use bcr_wdc_utils::keys;
use bitcoin::bip32::{self as btc32};
// ----- local modules
// ----- local imports
use crate::TStamp;

// ---------- Quote Keys Factory
#[derive(Clone)]
pub struct Factory {
    master: btc32::Xpriv,
    derivation: btc32::DerivationPath,
    unit: cashu::CurrencyUnit,
}

impl Factory {
    pub const MAX_ORDER: u8 = 32;
    pub const CURRENCY_UNIT: &'static str = "crsat";

    pub fn new(seed: &[u8], derivation: btc32::DerivationPath) -> Self {
        let master =
            btc32::Xpriv::new_master(bitcoin::Network::Bitcoin, seed).expect("bitcoin FAIL");
        Self {
            master,
            derivation,
            unit: cashu::CurrencyUnit::Custom(String::from(Self::CURRENCY_UNIT)),
        }
    }

    pub fn generate(&self, expire: TStamp) -> keys::KeysetEntry {
        let tstamp = expire.timestamp() as u64;
        let u16_0 = (tstamp as u16) as u32;
        let u16_1 = ((tstamp >> 16) as u16) as u32;
        let u16_2 = ((tstamp >> 32) as u16) as u32;
        let u16_3 = ((tstamp >> 48) as u16) as u32;

        let extension = [
            btc32::ChildNumber::from_hardened_idx(u16_3).unwrap(),
            btc32::ChildNumber::from_hardened_idx(u16_2).unwrap(),
            btc32::ChildNumber::from_hardened_idx(u16_1).unwrap(),
            btc32::ChildNumber::from_hardened_idx(u16_0).unwrap(),
        ];
        let path = self.derivation.extend(extension);
        let secp = secp256k1::global::SECP256K1;
        let xpriv = self
            .master
            .derive_priv(secp, &path)
            .expect("bitcoin::derive_priv unexpected error");
        let denominations: Vec<u64> = (0..Self::MAX_ORDER).map(|i| 2_u64.pow(i as u32)).collect();
        let keyset = cashu::MintKeySet::generate(
            secp,
            xpriv,
            self.unit.clone(),
            &denominations,
            Some(expire.timestamp() as u64),
            KeySetVersion::Version01,
        );

        let info = MintKeySetInfo {
            id: keyset.id,
            unit: keyset.unit.clone(),
            active: true,
            valid_from: chrono::Utc::now().timestamp() as u64,
            final_expiry: keyset.final_expiry,
            derivation_path: path,
            derivation_path_index: None,
            max_order: Self::MAX_ORDER,
            amounts: denominations,
            input_fee_ppk: 0,
        };
        (info, keyset)
    }
}
