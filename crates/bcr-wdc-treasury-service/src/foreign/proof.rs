// ----- standard library imports
use std::collections::HashMap;
// ----- extra library imports
use bcr_common::cashu::{self, nut10 as cdk10};
use bitcoin::hashes::sha256::Hash as Sha256Hash;
// ----- local imports
use crate::{
    error::{Error, Result},
    foreign::{ClowderClient, KeysClient},
    TStamp,
};

// ----- end imports

pub fn extract_hash_timelock_from_htlc(p: &cashu::Proof) -> Result<(Sha256Hash, TStamp)> {
    let Ok(secret) = cdk10::Secret::try_from(p.secret.clone()) else {
        return Err(Error::InvalidInput(String::from("no spending condition")));
    };
    let Ok(conditions) = cashu::SpendingConditions::try_from(secret) else {
        return Err(Error::InvalidInput(String::from("no spending condition")));
    };
    let cashu::SpendingConditions::HTLCConditions { data, conditions } = conditions else {
        return Err(Error::InvalidInput(String::from("no HTLC conditions")));
    };
    let Some(cashu::Conditions { locktime, .. }) = conditions else {
        return Err(Error::InvalidInput(String::from("no HTLC side-conditions")));
    };
    let Some(locktime) = locktime else {
        return Err(Error::InvalidInput(String::from("no HTLC time tag")));
    };
    let locktime = TStamp::from_timestamp_secs(locktime as i64)
        .ok_or(Error::InvalidInput(String::from("invalid HTLC time tag")))?;
    Ok((data, locktime))
}

// bytes secret estimate
const ONLINE_EXCHANGE_SECRET_SIZE: u64 = 450;

pub async fn generate_online_exchange_htlc_proofs(
    inputs: &[cashu::Proof],
    locktime: TStamp,
    hash: Sha256Hash,
    pk: cashu::PublicKey,
    foreign_id: secp256k1::PublicKey,
    clowdercl: &dyn ClowderClient,
    keycl: &dyn KeysClient,
    now: TStamp,
) -> Result<Vec<cashu::Proof>> {
    // generate premints conditions
    let conditions = cashu::Conditions::new(
        Some(locktime.timestamp() as u64),
        Some(vec![pk]),
        None,
        None,
        None,
        None,
    )?;
    let spending_conds =
        cashu::SpendingConditions::new_htlc_hash(&hash.to_string(), Some(conditions))?;
    // estimate fees for foreign swap of htlc proofs
    let mut foreign_kinfos: HashMap<cashu::Id, cashu::KeySetInfo> = HashMap::new();
    let mut max_fee_rate = 0;
    let mut totals: HashMap<cashu::Id, cashu::Amount> = HashMap::new();
    for p in inputs {
        if !foreign_kinfos.contains_key(&p.keyset_id) {
            let foreign_kinfo = clowdercl.get_keyset_info(&foreign_id, &p.keyset_id).await?;
            foreign_kinfos.insert(foreign_kinfo.id, foreign_kinfo);
        }
        let fee_rate = foreign_kinfos.get(&p.keyset_id).unwrap().input_fee_ppk;
        max_fee_rate = std::cmp::max(max_fee_rate, fee_rate);
        *totals.entry(p.keyset_id).or_insert(cashu::Amount::ZERO) += p.amount;
    }
    let total_fees = (inputs.len() as u64 * ONLINE_EXCHANGE_SECRET_SIZE * max_fee_rate)
        .div_ceil(bcr_common::core::swap::FEE_RATE_PPK_MULTIPLIER);
    let mut total_fees = cashu::Amount::from(total_fees);
    // update the total map discounting fees from older to newer keyset
    let mut foreign_kids = totals.keys().cloned().collect::<Vec<_>>();
    foreign_kids.sort_by_key(|k| {
        foreign_kinfos
            .get(k)
            .unwrap()
            .final_expiry
            .unwrap_or_default()
    });
    for foreign_kid in foreign_kids {
        if totals.get(&foreign_kid).unwrap() <= &total_fees {
            let amount = totals.remove(&foreign_kid);
            total_fees -= amount.unwrap_or_default();
        } else {
            *totals.get_mut(&foreign_kid).unwrap() -= total_fees;
            total_fees = cashu::Amount::ZERO;
            break;
        }
    }
    if total_fees > cashu::Amount::ZERO {
        return Err(Error::InvalidInput(format!(
            "inputs do not cover fees {total_fees}"
        )));
    }
    let mut proofs = Vec::with_capacity(inputs.len());
    for (foreign_kid, amount) in totals {
        let foreign_kinfo = foreign_kinfos.get(&foreign_kid).unwrap();
        let foreign_expiry = foreign_kinfo.final_expiry.unwrap_or_default();
        let foreign_expiration = chrono::DateTime::from_timestamp(foreign_expiry as i64, 0)
            .expect("final_expiry <--> chrono::Datetime");
        let kinfo = keycl
            .get_keyset_with_expiration(foreign_expiration.date_naive(), now)
            .await?;
        let premint = cashu::PreMintSecrets::with_conditions(
            kinfo.id,
            amount,
            &cashu::amount::SplitTarget::None,
            &spending_conds,
        )?;
        let keys = keycl.get_keyset(kinfo.id).await?;
        let blinds = premint.blinded_messages();
        let signatures = keycl.sign(&blinds).await?;
        let (rs, secrets) = premint
            .secrets
            .into_iter()
            .map(|pre| (pre.r, pre.secret))
            .unzip();
        let prfs = cashu::dhke::construct_proofs(signatures, rs, secrets, &keys.keys)?;
        proofs.extend(prfs);
    }
    Ok(proofs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_common::{core, core_tests};
    use bitcoin::hashes::{sha256::Hash as Sha256Hash, Hash};

    #[test]
    fn test_online_htlc_secret_size() {
        // this test is to check that the estimate of the secret size for online exchange HTLC proofs is sufficient
        // the actual size of the secret can be larger than the estimate, but it should not be smaller
        let hash = Sha256Hash::hash(b"test");
        let pk = cashu::PublicKey::from(core::generate_random_keypair().public_key());
        let refund_pk = cashu::PublicKey::from(core::generate_random_keypair().public_key());
        let conditions = cashu::Conditions::new(
            Some(chrono::Utc::now().timestamp() as u64),
            Some(vec![pk]),
            Some(vec![refund_pk]),
            Some(1),
            None,
            Some(1),
        )
        .unwrap();
        let spending_conds =
            cashu::SpendingConditions::new_htlc_hash(&hash.to_string(), Some(conditions)).unwrap();
        let (kinfo, _) = core_tests::generate_random_ecash_keyset();
        let premint = cashu::PreMintSecrets::with_conditions(
            kinfo.id,
            cashu::Amount::from(1000),
            &cashu::amount::SplitTarget::None,
            &spending_conds,
        )
        .unwrap();
        let secret_size = premint.secrets.first().unwrap().secret.as_bytes().len() as u64;
        assert!(secret_size <= ONLINE_EXCHANGE_SECRET_SIZE);
        assert!(secret_size > ONLINE_EXCHANGE_SECRET_SIZE * 9 / 10);
    }
}
