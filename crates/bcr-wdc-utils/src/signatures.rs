// ----- standard library imports
use std::collections::HashSet;
// ----- extra library imports
use bcr_common::{cashu, core::signature as core_signature};
use thiserror::Error;
// ----- local imports
use crate::keys as keys_utils;

// ----- end imports

pub type ChecksResult<T> = std::result::Result<T, ChecksError>;
#[derive(Debug, Error)]
pub enum ChecksError {
    #[error("vector is empty")]
    Empty,
    #[error("element with invalid amount: {0}")]
    InvalidAmount(String),
    #[error("non-unique elements")]
    NonUnique,
    #[error("spending conditions: {0}")]
    SpendingConditions(#[from] core_signature::ECashSignatureError),
}

pub fn basic_blinds_checks(blinds: &[cashu::BlindedMessage]) -> ChecksResult<()> {
    let zero_inputs = blinds
        .iter()
        .any(|output| output.amount == cashu::Amount::ZERO);
    if zero_inputs {
        return Err(ChecksError::InvalidAmount(String::from("zero amount")));
    }
    let denominations: Vec<cashu::Amount> = keys_utils::default_denominations()
        .into_iter()
        .map(cashu::Amount::from)
        .collect();
    for output in blinds {
        if !denominations.contains(&output.amount) {
            let msg = format!("invalid denomination: {}", output.amount);
            return Err(ChecksError::InvalidAmount(msg));
        }
    }
    let max_amount = cashu::Amount::from(bitcoin::Amount::MAX_MONEY.to_sat());
    let total = blinds.iter().try_fold(cashu::Amount::ZERO, |acc, output| {
        acc.checked_add(output.amount)
    });
    if total.is_none() || total.unwrap() > max_amount {
        return Err(ChecksError::InvalidAmount(String::from(
            "total amount overflow",
        )));
    }
    let unique_blinds: HashSet<_> = blinds.iter().map(|p| p.blinded_secret).collect();
    if unique_blinds.len() != blinds.len() {
        return Err(ChecksError::NonUnique);
    }
    Ok(())
}

pub fn basic_proofs_checks(proofs: &[cashu::Proof]) -> ChecksResult<()> {
    if proofs.is_empty() {
        return Err(ChecksError::Empty);
    }
    let zero_inputs = proofs
        .iter()
        .any(|output| output.amount == cashu::Amount::ZERO);
    if zero_inputs {
        return Err(ChecksError::InvalidAmount(String::from("zero amount")));
    }
    let denominations = keys_utils::default_denominations();
    let unknown_amount = proofs
        .iter()
        .any(|output| !denominations.contains(&output.amount.to_u64()));
    if unknown_amount {
        return Err(ChecksError::InvalidAmount(String::from("unknown_amount")));
    }
    let total = proofs.iter().try_fold(cashu::Amount::ZERO, |acc, output| {
        acc.checked_add(output.amount)
    });
    let max_amount = cashu::Amount::from(bitcoin::Amount::MAX_MONEY.to_sat());
    if total.is_none() || total.unwrap() > max_amount {
        return Err(ChecksError::InvalidAmount(String::from(
            "total amount overflow",
        )));
    }
    let unique_proofs: HashSet<_> = proofs.iter().map(|p| p.secret.clone()).collect();
    if unique_proofs.len() != proofs.len() {
        return Err(ChecksError::NonUnique);
    }
    // 4. p2pk / htlc
    for proof in proofs {
        core_signature::verify_spending_conditions(proof)?;
    }
    Ok(())
}

pub fn basic_fingerprints_checks(fps: &[core_signature::ProofFingerprint]) -> ChecksResult<()> {
    // 1. no empty fingerprint
    if fps.is_empty() {
        return Err(ChecksError::Empty);
    }
    // 2. no zero amounts
    let zero_inputs = fps
        .iter()
        .any(|output| output.amount == cashu::Amount::ZERO);
    if zero_inputs {
        return Err(ChecksError::InvalidAmount(String::from("zero amount")));
    }
    // 3. unique fingerprints
    let unique_fps: HashSet<_> = fps.iter().map(|p| p.y).collect();
    if unique_fps.len() != fps.len() {
        return Err(ChecksError::NonUnique);
    }
    Ok(())
}

#[cfg(any(feature = "test-utils", test))]
pub mod test_utils {
    use super::*;
    use crate::keys::test_utils::{generate_blind, publics};
    use bcr_common::{core_tests, ecash, wire::keys as wire_keys};
    use cashu::{secret, Id};

    pub fn random_schnorr_signature() -> bitcoin::secp256k1::schnorr::Signature {
        use rand::Rng;
        let mut sl = [0u8; bitcoin::secp256k1::constants::SCHNORR_SIGNATURE_SIZE];
        rand::thread_rng().fill(&mut sl[..]);
        bitcoin::secp256k1::schnorr::Signature::from_slice(&sl).unwrap()
    }

    pub fn generate_blinds(
        id: Id,
        amounts: &[cashu::Amount],
    ) -> Vec<(cashu::BlindedMessage, secret::Secret, cashu::SecretKey)> {
        let mut blinds: Vec<(cashu::BlindedMessage, secret::Secret, cashu::SecretKey)> = Vec::new();
        for amount in amounts {
            blinds.push(generate_blind(id, *amount));
        }
        blinds
    }

    pub fn generate_signatures(
        keyset_id: cashu::Id,
        amounts: &[cashu::Amount],
    ) -> Vec<cashu::BlindSignature> {
        let mut signatures: Vec<cashu::BlindSignature> = Vec::new();
        for amount in amounts {
            signatures.push(cashu::BlindSignature {
                keyset_id,
                amount: *amount,
                c: publics()[0],
                dleq: None,
            });
        }
        signatures
    }

    pub fn generate_fingerprints(
        keyset: &ecash::MintKeySet,
        amounts: &[cashu::Amount],
    ) -> Vec<wire_keys::ProofFingerprint> {
        let proofs = core_tests::generate_random_ecash_proofs(keyset, amounts);
        proofs
            .into_iter()
            .map(wire_keys::ProofFingerprint::try_from)
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    }

    pub fn verify_signatures_data(
        keyset: &ecash::MintKeySet,
        signatures: impl std::iter::IntoIterator<Item = (cashu::BlindedMessage, cashu::BlindSignature)>,
    ) -> bool {
        for (msg, sig) in signatures.into_iter() {
            let kid = cashu::Id::from(keyset.id);
            if msg.keyset_id != kid || sig.keyset_id != kid {
                return false;
            }
            if msg.amount != sig.amount {
                return false;
            }

            let keypair = keyset.keys.get(&sig.amount);
            if keypair.is_none() {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::test_utils::*;
    use super::*;
    use bcr_common::core_tests;

    #[test]
    fn basic_checks_empty_slice() {
        let blinds = vec![];
        assert!(matches!(basic_blinds_checks(&blinds), Ok(())));
        let proofs = vec![];
        assert!(matches!(
            basic_proofs_checks(&proofs),
            Err(ChecksError::Empty)
        ));
    }

    #[test]
    fn basic_checks_zero_amount() {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts = vec![cashu::Amount::from(64), cashu::Amount::from(2)];
        let mut blinds: Vec<_> = generate_blinds(keyset.id.into(), &amounts)
            .into_iter()
            .map(|(blind, _, _)| blind)
            .collect();
        blinds[0].amount = cashu::Amount::ZERO;
        assert!(matches!(
            basic_blinds_checks(&blinds),
            Err(ChecksError::InvalidAmount(_))
        ));
        let mut proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        proofs[0].amount = cashu::Amount::ZERO;
        assert!(matches!(
            basic_proofs_checks(&proofs),
            Err(ChecksError::InvalidAmount(_))
        ));
    }

    #[test]
    fn basic_checks_unique() {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts = vec![cashu::Amount::from(64), cashu::Amount::from(8)];
        let mut blinds: Vec<_> = generate_blinds(keyset.id.into(), &amounts)
            .into_iter()
            .map(|(blind, _, _)| blind)
            .collect();
        blinds.push(blinds[0].clone());
        assert!(matches!(
            basic_blinds_checks(&blinds),
            Err(ChecksError::NonUnique)
        ));
        let mut proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        proofs.push(proofs[0].clone());
        assert!(matches!(
            basic_proofs_checks(&proofs),
            Err(ChecksError::NonUnique)
        ));
    }
}
