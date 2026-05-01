// ----- standard library imports
// ----- extra library imports
use bcr_common::{
    cashu::{self, nut10 as cdk10},
    core::signature::unblind_ecash_signature,
};
use bitcoin::hashes::sha256::Hash as Sha256Hash;
// ----- local imports
use crate::{
    error::{Error, Result},
    foreign::{ClowderClient, ForeignClient, KeysClient},
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

/// check that all proofs:
/// - are unspent
/// - have same keyset_id, same htlc hash, same locktime
/// - perform check_htlc_foreign_proof
pub async fn check_htlc_foreign_proofs(
    issuer: cashu::PublicKey,
    proofs: &[cashu::Proof],
    mintcl: &dyn ForeignClient,
    clwdcl: &dyn ClowderClient,
) -> Result<(Sha256Hash, TStamp)> {
    if proofs.is_empty() {
        return Err(Error::InvalidInput(String::from("no proofs")));
    }
    clwdcl.check_htlc_proofs(issuer, proofs.to_vec()).await?;

    let fingerprints: Vec<cashu::PublicKey> = proofs
        .iter()
        .map(|p| p.y())
        .collect::<std::result::Result<_, _>>()?;
    let states = mintcl.check_state(fingerprints).await?;
    let unspent = states
        .iter()
        .all(|s| matches!(s.state, cashu::nut07::State::Unspent));
    if !unspent {
        return Err(Error::InvalidInput(String::from(
            "One or more proofs are not unspent",
        )));
    }
    let (hash, locktime) = extract_hash_timelock_from_htlc(&proofs[0])?;
    Ok((hash, locktime))
}

fn generate_htlc_conditions(
    locktime: Option<TStamp>,
    hash: Sha256Hash,
    pk: cashu::PublicKey,
) -> Result<cashu::SpendingConditions> {
    let conditions = cashu::Conditions::new(
        locktime.map(|t| t.timestamp() as u64),
        Some(vec![pk]),
        None,
        None,
        None,
        None,
    )?;
    let spending_conds =
        cashu::SpendingConditions::new_htlc_hash(&hash.to_string(), Some(conditions))?;
    Ok(spending_conds)
}

pub async fn generate_htlc_proofs(
    amount: cashu::Amount,
    locktime: Option<TStamp>,
    hash: Sha256Hash,
    pk: cashu::PublicKey,
    keyset: &cashu::KeySet,
    keycl: &dyn KeysClient,
) -> Result<Vec<cashu::Proof>> {
    let spending_conds = generate_htlc_conditions(locktime, hash, pk)?;
    let premints = cashu::PreMintSecrets::with_conditions(
        keyset.id,
        amount,
        &cashu::amount::SplitTarget::None,
        &spending_conds,
    )?;
    let blinds = premints.blinded_messages();
    let signatures = keycl.sign(&blinds).await?;
    let mut proofs = Vec::with_capacity(signatures.len());
    for (sig, pre) in signatures.into_iter().zip(premints.iter()) {
        let proof = unblind_ecash_signature(keyset, pre.clone(), sig)?;
        proofs.push(proof);
    }
    Ok(proofs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_common::core_tests;
    use bitcoin::hashes::Hash;
    use std::collections::HashSet;

    #[test]
    fn generate_htlc_premints_have_unique_y_and_correct_hash() {
        let hash = Sha256Hash::hash(b"test_preimage");
        let kp = core_tests::generate_random_keypair();
        let pk = cashu::PublicKey::from(kp.public_key());
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amount = cashu::Amount::from(1023);
        let locktime = Some(chrono::Utc::now() + chrono::TimeDelta::hours(1));

        let spending_conds = generate_htlc_conditions(locktime, hash, pk).unwrap();
        let premints = cashu::PreMintSecrets::with_conditions(
            keyset.id,
            amount,
            &cashu::amount::SplitTarget::None,
            &spending_conds,
        )
        .unwrap();

        assert!(premints.len() == 10);

        let mut ys = HashSet::new();
        for pm in premints.iter() {
            let proof = cashu::Proof {
                keyset_id: keyset.id,
                amount: pm.amount,
                secret: pm.secret.clone(),
                c: cashu::PublicKey::from(kp.public_key()), // dummy
                witness: None,
                dleq: None,
            };
            let y = proof.y().unwrap();
            assert!(ys.insert(y), "duplicate Y found");

            // Verify HTLC hash
            let (extracted_hash, _) = extract_hash_timelock_from_htlc(&proof).unwrap();
            assert_eq!(extracted_hash, hash);
        }
    }
}
