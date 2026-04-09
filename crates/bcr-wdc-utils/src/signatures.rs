// ----- standard library imports
// ----- extra library imports
use bcr_common::cashu;
use itertools::Itertools;
use thiserror::Error;
// ----- local imports

// ----- end imports

pub type ChecksResult<T> = std::result::Result<T, ChecksError>;
#[derive(Debug, Error)]
pub enum ChecksError {
    #[error("vector is empty")]
    Empty,
    #[error("element with zero amount not allowed")]
    ZeroAmount,
    #[error("non-unique elements")]
    NonUnique,
}

pub fn basic_blinds_checks(blinds: &[cashu::BlindedMessage]) -> ChecksResult<()> {
    // 1. no empty blinds
    if blinds.is_empty() {
        return Err(ChecksError::Empty);
    }
    // 2. no zero amounts
    let zero_inputs = blinds
        .iter()
        .any(|output| output.amount == cashu::Amount::ZERO);
    if zero_inputs {
        return Err(ChecksError::ZeroAmount);
    }
    // 3. unique blinds
    let unique_blinds: Vec<_> = blinds.iter().map(|p| p.blinded_secret).unique().collect();
    if unique_blinds.len() != blinds.len() {
        return Err(ChecksError::NonUnique);
    }
    Ok(())
}
pub fn basic_proofs_checks(proofs: &[cashu::Proof]) -> ChecksResult<()> {
    // 1. no empty proofs
    if proofs.is_empty() {
        return Err(ChecksError::Empty);
    }
    // 2. no zero amounts
    let zero_inputs = proofs
        .iter()
        .any(|output| output.amount == cashu::Amount::ZERO);
    if zero_inputs {
        return Err(ChecksError::ZeroAmount);
    }
    // 3. unique proofs
    let unique_proofs: Vec<_> = proofs.iter().map(|p| p.secret.clone()).unique().collect();
    if unique_proofs.len() != proofs.len() {
        return Err(ChecksError::NonUnique);
    }
    Ok(())
}

#[cfg(any(feature = "test-utils", test))]
pub mod test_utils {
    use super::*;
    use crate::keys::test_utils::{generate_blind, publics};
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
        keyset: &cashu::MintKeySet,
        amounts: &[cashu::Amount],
    ) -> Vec<cashu::BlindSignature> {
        let mut signatures: Vec<cashu::BlindSignature> = Vec::new();
        for amount in amounts {
            signatures.push(cashu::BlindSignature {
                keyset_id: keyset.id,
                amount: *amount,
                c: publics()[0],
                dleq: None,
            });
        }
        signatures
    }

    pub fn verify_signatures_data(
        keyset: &cashu::MintKeySet,
        signatures: impl std::iter::IntoIterator<Item = (cashu::BlindedMessage, cashu::BlindSignature)>,
    ) -> bool {
        for (msg, sig) in signatures.into_iter() {
            if msg.keyset_id != keyset.id || sig.keyset_id != keyset.id {
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
    use crate::keys::test_utils::generate_keyset;
    use bcr_common::core_tests;

    #[test]
    fn basic_checks_empty_slice() {
        let blinds = vec![];
        assert!(matches!(
            basic_blinds_checks(&blinds),
            Err(ChecksError::Empty)
        ));
        let proofs = vec![];
        assert!(matches!(
            basic_proofs_checks(&proofs),
            Err(ChecksError::Empty)
        ));
    }

    #[test]
    fn basic_checks_zero_amount() {
        let (_, keyset) = generate_keyset();
        let amounts = vec![cashu::Amount::from(64), cashu::Amount::from(2)];
        let mut blinds: Vec<_> = generate_blinds(keyset.id, &amounts)
            .into_iter()
            .map(|(blind, _, _)| blind)
            .collect();
        blinds[0].amount = cashu::Amount::ZERO;
        assert!(matches!(
            basic_blinds_checks(&blinds),
            Err(ChecksError::ZeroAmount)
        ));
        let mut proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        proofs[0].amount = cashu::Amount::ZERO;
        assert!(matches!(
            basic_proofs_checks(&proofs),
            Err(ChecksError::ZeroAmount)
        ));
    }

    #[test]
    fn basic_checks_unique() {
        let (_, keyset) = generate_keyset();
        let amounts = vec![cashu::Amount::from(64), cashu::Amount::from(8)];
        let mut blinds: Vec<_> = generate_blinds(keyset.id, &amounts)
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
