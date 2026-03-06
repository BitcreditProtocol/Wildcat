// ----- standard library imports
// ----- extra library imports
use bcr_common::cashu::{self, nut02::KeySetVersion};
use bcr_common::cdk_common::mint::MintKeySetInfo;
use bcr_wdc_utils::keys;
use bitcoin::{
    bip32 as btc32,
    hashes::{sha256::Hash as Sha256, Hash},
};
// ----- local imports
use crate::TStamp;

// ---------- Quote Keys Factory
#[derive(Clone)]
pub struct Factory {
    master: btc32::Xpriv,
    derivation: btc32::DerivationPath,
}

impl Factory {
    pub const MAX_ORDER: u8 = 32;

    pub fn new(seed: &[u8], derivation: btc32::DerivationPath) -> Self {
        let master =
            btc32::Xpriv::new_master(bitcoin::Network::Bitcoin, seed).expect("bitcoin FAIL");
        Self { master, derivation }
    }
    pub fn generate(
        &self,
        unit: cashu::CurrencyUnit,
        now: TStamp,
        expiration: Option<TStamp>,
        fees_ppk: u64,
    ) -> keys::KeysetEntry {
        // sha of info.unit.to_string()
        let unit_hash = Sha256::hash(unit.to_string().as_bytes());
        // we take least significant 31 bits of the hash as u32
        let unit_idx = ((unit_hash[0] as u32) << 24
            | (unit_hash[1] as u32) << 16
            | (unit_hash[2] as u32) << 8
            | (unit_hash[3] as u32))
            & 0x7FFFFFFF;
        let expire = expiration.map(|e| e.timestamp().max(0) as u64);
        let expire_tstamp = expire.unwrap_or_default();
        // concatenate now and expiration_tstamp as vec<u8> of length 16 (8 bytes for each)
        let time_vec: Vec<u8> = std::iter::chain(
            now.timestamp().to_be_bytes().into_iter(),
            expire_tstamp.to_be_bytes().into_iter(),
        )
        .collect();
        // sha of the concatenated vec
        let time_hash = Sha256::hash(&time_vec);
        // we take least significant 31 bits of the hash as u32
        let time_idx_1 = ((time_hash[0] as u32) << 24
            | (time_hash[1] as u32) << 16
            | (time_hash[2] as u32) << 8
            | (time_hash[3] as u32))
            & 0x7FFFFFFF;
        // we take least significant 31 bits of the next 32 bits from the hash
        let time_idx_2 = ((time_hash[4] as u32) << 24
            | (time_hash[5] as u32) << 16
            | (time_hash[6] as u32) << 8
            | (time_hash[7] as u32))
            & 0x7FFFFFFF;
        let extension = [
            btc32::ChildNumber::from_hardened_idx(unit_idx).unwrap(),
            btc32::ChildNumber::from_hardened_idx(time_idx_1).unwrap(),
            btc32::ChildNumber::from_hardened_idx(time_idx_2).unwrap(),
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
            unit,
            &denominations,
            expire,
            KeySetVersion::Version01,
        );
        tracing::info!(
            "new keyset generated: {}, {now}, {}, {fees_ppk} ==> {}",
            keyset.unit,
            expiration.unwrap_or_default(),
            keyset.id
        );
        let info = MintKeySetInfo {
            id: keyset.id,
            unit: keyset.unit.clone(),
            active: true,
            valid_from: now.timestamp().max(0) as u64,
            final_expiry: keyset.final_expiry,
            derivation_path: path,
            derivation_path_index: None,
            max_order: Self::MAX_ORDER,
            amounts: denominations,
            input_fee_ppk: fees_ppk,
        };
        (info, keyset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_factory_generate() {
        let seed = [0u8; 32];
        let derivation = btc32::DerivationPath::default();
        let factory = Factory::new(&seed, derivation);
        let unit1 = cashu::CurrencyUnit::Sat;
        let now = chrono::DateTime::from_timestamp(1_000_000, 0).unwrap();
        let expire1 = chrono::DateTime::from_timestamp(2_000_000, 0).unwrap();
        let (info1, _) = factory.generate(unit1.clone(), now, Some(expire1), 100);
        assert_eq!(info1.unit, unit1);
        assert_eq!(info1.final_expiry, Some(expire1.timestamp() as u64));
        // different unit
        let unit2 = cashu::CurrencyUnit::Eur;
        let (info2, _) = factory.generate(unit2.clone(), now, Some(expire1), 0);
        assert_eq!(info2.unit, unit2);
        assert_ne!(info2.id, info1.id);
        // different expiration
        let expire3 = chrono::DateTime::from_timestamp(3_000_000, 0).unwrap();
        let (info3, _) = factory.generate(unit1.clone(), now, Some(expire3), 0);
        assert_eq!(info3.final_expiry, Some(expire3.timestamp() as u64));
        assert_ne!(info3.id, info1.id);
        assert_ne!(info3.id, info2.id);
        // different now
        let now4 = chrono::DateTime::from_timestamp(1_500_000, 0).unwrap();
        let (info4, _) = factory.generate(unit1.clone(), now4, Some(expire1), 0);
        assert_eq!(info4.valid_from, now4.timestamp() as u64);
        assert_ne!(info4.id, info1.id);
        assert_ne!(info4.id, info2.id);
        assert_ne!(info4.id, info3.id);
    }
}
