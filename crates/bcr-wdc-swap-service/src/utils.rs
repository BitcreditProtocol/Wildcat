// ----- standard library imports
// ----- extra library imports
use bcr_wdc_keys::test_utils::generate_blind;
use cashu::Amount as cdk_Amount;
use cashu::dhke as cdk_dhke;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut01 as cdk01;
use cashu::nuts::nut02 as cdk02;
use cashu::secret as cdk_secret;
// ----- local imports

pub fn generate_proofs(keyset: &cdk02::MintKeySet, amounts: &[cdk_Amount]) -> Vec<cdk00::Proof> {
    let mut proofs: Vec<cdk00::Proof> = Vec::new();
    for amount in amounts {
        let keypair = keyset.keys.get(amount).expect("keys for amount");
        let secret = cdk_secret::Secret::new(rand::random::<u64>().to_string());
        let (b_, r) =
            cdk_dhke::blind_message(secret.as_bytes(), None).expect("cdk_dhke::blind_message");
        let c_ = cdk_dhke::sign_message(&keypair.secret_key, &b_).expect("cdk_dhke::sign_message");
        let c = cdk_dhke::unblind_message(&c_, &r, &keypair.public_key).expect("unblind_message");
        proofs.push(cdk00::Proof::new(*amount, keyset.id, secret, c));
    }
    proofs
}

pub fn generate_blinds(
    keyset: &cdk02::MintKeySet,
    amounts: &[cdk_Amount],
) -> Vec<(cdk00::BlindedMessage, cdk_secret::Secret, cdk01::SecretKey)> {
    let mut blinds: Vec<(cdk00::BlindedMessage, cdk_secret::Secret, cdk01::SecretKey)> = Vec::new();
    for amount in amounts {
        blinds.push(generate_blind(keyset, amount));
    }
    blinds
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
