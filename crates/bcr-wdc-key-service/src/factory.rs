// ----- standard library imports
use std::collections::BTreeMap;
// ----- extra library imports
use bcr_wdc_utils::keys;
use bitcoin::bip32 as btc32;
use cashu::{nut00 as cdk00, nut01 as cdk01, nut02 as cdk02, Amount};
use cdk_common::mint as cdk_mint;
// ----- local modules
// ----- local imports
use crate::TStamp;

// ---------- Quote Keys Factory
#[derive(Clone)]
pub struct Factory {
    master: btc32::Xpriv,
    derivation: btc32::DerivationPath,
    unit: cdk00::CurrencyUnit,
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
            unit: cdk00::CurrencyUnit::Custom(String::from(Self::CURRENCY_UNIT)),
        }
    }

    pub fn generate(&self, quote: uuid::Uuid, expire: TStamp) -> keys::KeysetEntry {
        let path = keys::extend_path_from_uuid(quote, &self.derivation);
        let set = generate_mintkeyset(
            self.master,
            Self::MAX_ORDER,
            self.unit.clone(),
            path.clone(),
            Some(expire.timestamp() as u64),
        );
        let info = cdk_mint::MintKeySetInfo {
            id: set.id,
            unit: self.unit.clone(),
            active: false,
            valid_from: chrono::Utc::now().timestamp() as u64,
            final_expiry: Some(expire.timestamp() as u64),
            derivation_path: path,
            derivation_path_index: None,
            max_order: Self::MAX_ORDER,
            input_fee_ppk: 0,
        };
        (info, set)
    }
}

/// rework of `cashu::nut02::MintKeySet::generate_from_xpriv`
/// from the given master xpriv, we derive <max_order> children
/// one for each amount 2^0, 2^1, ..., 2^max_order
fn generate_mintkeyset(
    master: btc32::Xpriv,
    max_order: u8,
    unit: cdk00::CurrencyUnit,
    path: btc32::DerivationPath,
    final_expiry: Option<u64>,
) -> cdk02::MintKeySet {
    let secp = secp256k1::global::SECP256K1;
    let xpriv = master.derive_priv(secp, &path).expect("RNG busted");
    let mut map = BTreeMap::new();
    for i in 0..max_order {
        let amount = Amount::from(2_u64.pow(i as u32));
        let secret_key = xpriv
            .derive_priv(
                secp,
                &[btc32::ChildNumber::from_normal_idx(i as u32).expect("order is valid index")],
            )
            .expect("RNG busted")
            .private_key;
        let public_key = secret_key.public_key(secp);
        map.insert(
            amount,
            cdk01::MintKeyPair {
                secret_key: secret_key.into(),
                public_key: public_key.into(),
            },
        );
    }

    let keys = cdk01::MintKeys::new(map);
    cdk02::MintKeySet {
        id: (&keys).into(),
        unit,
        keys,
        final_expiry,
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

        let quote = uuid::Uuid::from_u128(0);
        let maturity = chrono::DateTime::parse_from_rfc3339("2021-01-01T00:00:00Z")
            .unwrap()
            .to_utc();

        let factory = Factory::new(&seed, btc32::DerivationPath::default());

        let (_, keyset) = factory.generate(quote, maturity);
        let key = &keyset.keys[&cdk_Amount::from(1_u64)];
        // m/0/0/0/0/0
        assert_eq!(
            key.public_key.to_hex(),
            "027668145a12f96edab70d9c68b18440fe07e197355be727a8be9e1f09fb2953d4"
        );
        let key = &keyset.keys[&cdk_Amount::from(32_u64)];
        // m/0/0/0/0/5
        assert_eq!(
            key.public_key.to_hex(),
            "02c5bb7222ca5dd5251fee6bd753fa36210989a4f2174769df9ae6bc16a0f22562"
        );
    }
}
