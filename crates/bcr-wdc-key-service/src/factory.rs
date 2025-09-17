// ----- standard library imports
// ----- extra library imports
use bcr_wdc_utils::keys;
use bitcoin::bip32::{self as btc32};
use cashu::nut02::KeySetVersion;
use cdk_common::mint::MintKeySetInfo;
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
    /// amount of currency denominations, from 2^0 to 2^MAX_ORDER
    pub const MAX_ORDER: u8 = 20;
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
        dbg!(tstamp, u16_3, u16_2, u16_1, u16_0, extension);
        let path = self.derivation.extend(extension);
        let secp = secp256k1::global::SECP256K1;
        let xpriv = self
            .master
            .derive_priv(secp, &path)
            .expect("bitcoin::derive_priv unexpected error");
        let keyset = cashu::MintKeySet::generate(
            secp,
            xpriv,
            self.unit.clone(),
            Self::MAX_ORDER,
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
            input_fee_ppk: 0,
        };
        (info, keyset)
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use cashu::Amount as cdk_Amount;
    use std::str::FromStr;

    #[test]
    fn factory_generate() {
        let seed = bip39::Mnemonic::from_str("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").unwrap().to_seed("");
        // unixtimestamp -> 1609459200 -> 000000005fee6600 -> 0000 0000 5fee 6600 -> 0 0 24558 26112
        let maturity = chrono::DateTime::parse_from_rfc3339("2021-01-01T00:00:00Z")
            .unwrap()
            .to_utc();
        let factory = Factory::new(&seed, btc32::DerivationPath::default());
        let (_, keyset) = factory.generate(maturity);
        let key = &keyset.keys[&cdk_Amount::from(1_u64)];
        // m/0'/0'/24558'/26112'/0'
        assert_eq!(
            key.public_key.to_hex(),
            "023f9b71d4835213b5368c7c2caacd002e0e27fab263247b8afb4e62f77b94ba6a"
        );
        let key = &keyset.keys[&cdk_Amount::from(32_u64)];
        // m/0'/0'/24558'/26112'/5'
        assert_eq!(
            key.public_key.to_hex(),
            "0299e8b4649e47729fbb3d56bd62085843c0118340bd953459380a40c1494cfc90"
        );
    }
}
