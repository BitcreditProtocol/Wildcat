// ----- standard library imports
// ----- extra library imports
use cashu::{nut00 as cdk00, nut01 as cdk01, nut02 as cdk02, Amount};
use itertools::Itertools;
use thiserror::Error;
// ----- local imports

// ----- end imports
//
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

pub fn basic_blinds_checks(blinds: &[cdk00::BlindedMessage]) -> ChecksResult<()> {
    // 1. no empty blinds
    if blinds.is_empty() {
        return Err(ChecksError::Empty);
    }
    // 2. no zero amounts
    let zero_inputs = blinds.iter().any(|output| output.amount == Amount::ZERO);
    if zero_inputs {
        return Err(ChecksError::ZeroAmount);
    }
    // 3. unique blinds
    let unique_proofs: Vec<_> = blinds.iter().map(|p| p.blinded_secret).unique().collect();
    if unique_proofs.len() != blinds.len() {
        return Err(ChecksError::NonUnique);
    }
    Ok(())
}
pub fn basic_proofs_checks(proofs: &[cdk00::Proof]) -> ChecksResult<()> {
    // 1. no empty proofs
    if proofs.is_empty() {
        return Err(ChecksError::Empty);
    }
    // 2. no zero amounts
    let zero_inputs = proofs.iter().any(|output| output.amount == Amount::ZERO);
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
    use cashu::{dhke, secret};

    pub fn generate_proofs(keyset: &cdk02::MintKeySet, amounts: &[Amount]) -> Vec<cdk00::Proof> {
        let mut proofs: Vec<cdk00::Proof> = Vec::new();
        for amount in amounts {
            let keypair = keyset.keys.get(amount).expect("keys for amount");
            let secret = secret::Secret::new(rand::random::<u64>().to_string());
            let (b_, r) =
                dhke::blind_message(secret.as_bytes(), None).expect("cdk_dhke::blind_message");
            let c_ = dhke::sign_message(&keypair.secret_key, &b_).expect("cdk_dhke::sign_message");
            let c = dhke::unblind_message(&c_, &r, &keypair.public_key).expect("unblind_message");
            proofs.push(cdk00::Proof::new(*amount, keyset.id, secret, c));
        }
        proofs
    }

    pub fn generate_blinds(
        keyset: &cdk02::MintKeySet,
        amounts: &[Amount],
    ) -> Vec<(cdk00::BlindedMessage, secret::Secret, cdk01::SecretKey)> {
        let mut blinds: Vec<(cdk00::BlindedMessage, secret::Secret, cdk01::SecretKey)> = Vec::new();
        for amount in amounts {
            blinds.push(generate_blind(keyset.id, *amount));
        }
        blinds
    }

    pub fn generate_signatures(
        keyset: &cdk02::MintKeySet,
        amounts: &[Amount],
    ) -> Vec<cdk00::BlindSignature> {
        let mut signatures: Vec<cdk00::BlindSignature> = Vec::new();
        for amount in amounts {
            signatures.push(cdk00::BlindSignature {
                keyset_id: keyset.id,
                amount: *amount,
                c: publics()[0],
                dleq: None,
            });
        }
        signatures
    }

    pub fn verify_signatures_data(
        keyset: &cdk02::MintKeySet,
        signatures: impl std::iter::IntoIterator<Item = (cdk00::BlindedMessage, cdk00::BlindSignature)>,
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
        let amounts = vec![Amount::from(64), Amount::from(2)];

        let mut blinds: Vec<_> = generate_blinds(&keyset, &amounts)
            .into_iter()
            .map(|(blind, _, _)| blind)
            .collect();
        blinds[0].amount = Amount::ZERO;
        assert!(matches!(
            basic_blinds_checks(&blinds),
            Err(ChecksError::ZeroAmount)
        ));

        let mut proofs = generate_proofs(&keyset, &amounts);
        proofs[0].amount = Amount::ZERO;
        assert!(matches!(
            basic_proofs_checks(&proofs),
            Err(ChecksError::ZeroAmount)
        ));
    }

    #[test]
    fn basic_checks_unique() {
        let (_, keyset) = generate_keyset();
        let amounts = vec![Amount::from(64), Amount::from(8)];

        let mut blinds: Vec<_> = generate_blinds(&keyset, &amounts)
            .into_iter()
            .map(|(blind, _, _)| blind)
            .collect();
        blinds.push(blinds[0].clone());
        assert!(matches!(
            basic_blinds_checks(&blinds),
            Err(ChecksError::NonUnique)
        ));

        let mut proofs = generate_proofs(&keyset, &amounts);
        proofs.push(proofs[0].clone());
        assert!(matches!(
            basic_proofs_checks(&proofs),
            Err(ChecksError::NonUnique)
        ));
    }
}
