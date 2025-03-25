// ----- standard library imports
// ----- extra library imports
use bcr_wdc_keys as keys;
use bitcoin::bip32 as btc32;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut02 as cdk02;
use cdk_common::mint as cdk_mint;
// ----- local modules
// ----- local imports
use crate::TStamp;

// ---------- Quote Keys Factory
#[derive(Clone)]
pub struct Factory {
    ctx: bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>,
    xpriv: btc32::Xpriv,
    unit: cdk00::CurrencyUnit,
}

impl Factory {
    /// amount of currency denominations, from 2^0 to 2^MAX_ORDER
    pub const MAX_ORDER: u8 = 20;
    pub const CURRENCY_UNIT: &'static str = "crsat";

    pub fn new(seed: &[u8]) -> Self {
        Self {
            ctx: bitcoin::secp256k1::Secp256k1::new(),
            xpriv: btc32::Xpriv::new_master(bitcoin::Network::Bitcoin, seed).expect("bitcoin FAIL"),
            unit: cdk00::CurrencyUnit::Custom(String::from(Self::CURRENCY_UNIT)),
        }
    }

    pub fn generate(
        &self,
        keysetid: cdk02::Id,
        quote: uuid::Uuid,
        expire: TStamp,
    ) -> keys::KeysetEntry {
        let path = keys::generate_keyset_path(keysetid.into(), Some(quote));
        let keys = cdk02::MintKeySet::generate_from_xpriv(
            &self.ctx,
            self.xpriv,
            Self::MAX_ORDER,
            self.unit.clone(),
            path.clone(),
        )
        .keys;
        let info = cdk_mint::MintKeySetInfo {
            id: keysetid,
            unit: self.unit.clone(),
            active: false,
            valid_from: chrono::Utc::now().timestamp() as u64,
            valid_to: Some(expire.timestamp() as u64),
            derivation_path: path,
            derivation_path_index: None,
            max_order: Self::MAX_ORDER,
            input_fee_ppk: 0,
        };
        let set = cdk02::MintKeySet {
            id: keysetid,
            keys,
            unit: self.unit.clone(),
        };
        (info, set)
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

        let kid = cdk02::Id::from_bytes(&[0u8; 8]).unwrap();
        let quote = uuid::Uuid::from_u128(0);
        let maturity = chrono::DateTime::parse_from_rfc3339("2021-01-01T00:00:00Z")
            .unwrap()
            .to_utc();

        let factory = Factory::new(&seed);

        let (_, keyset) = factory.generate(kid, quote, maturity);
        // m/129372'/129534'/0'/927402239'/0'
        let key = &keyset.keys[&cdk_Amount::from(1_u64)];
        assert_eq!(
            key.public_key.to_hex(),
            "03287106d3d2f1df660f7c7764e39e98051bca0c95feb9604336e9744de88eac68"
        );
        // m/129372'/129534'/0'/927402239'/5'
        let key = &keyset.keys[&cdk_Amount::from(32_u64)];
        assert_eq!(
            key.public_key.to_hex(),
            "03c5b66986d15100d1c0b342e012da7a954c7040c13d514ebc3b282ffa3a54651f"
        );
    }
}
