use bcr_wdc_webapi::test_utils::generate_random_bill_enquire_request;
// ----- standard library imports
// ----- extra library imports
// ----- local modules
use bitcoin::secp256k1::Keypair;
use cashu::nuts::nut02 as cdk02;
use cashu::Amount;
// ----- end imports

pub fn random_ebill_request() -> (
    Keypair,
    bcr_wdc_webapi::quotes::EnquireRequest,
    bitcoin::secp256k1::schnorr::Signature,
) {
    let owner_key = bcr_wdc_utils::keys::test_utils::generate_random_keypair();
    let (request, signing_kp) = generate_random_bill_enquire_request(owner_key, None);

    let signature = bcr_wdc_utils::keys::schnorr_sign_borsh_msg_with_key(&request, &signing_kp)
        .expect("schnorr_sign_borsh_msg_with_key");

    (owner_key, request, signature)
}

pub fn get_amounts(mut targ: u64) -> Vec<u64> {
    // TODO see if there is an existing cashu implementation
    let mut coins = Vec::new();
    let mut bit_position = 0;
    while targ > 0 {
        if (targ & 1) == 1 {
            coins.push(1 << bit_position);
        }
        targ >>= 1;
        bit_position += 1;
    }
    coins
}

pub fn generate_blinds(
    keyset_id: cdk02::Id,
    amounts: &[Amount],
) -> Vec<(
    cashu::BlindedMessage,
    cashu::secret::Secret,
    cashu::SecretKey,
)> {
    let mut blinds = Vec::new();
    for amount in amounts {
        let blind = bcr_wdc_utils::keys::test_utils::generate_blind(keyset_id, *amount);
        blinds.push(blind);
    }
    blinds
}

#[cfg(test)]
mod tests {
    use crate::test_utils::get_amounts;
    #[test]
    fn test_get_amounts() {
        let amounts = get_amounts(1000);
        let sum = amounts.iter().sum::<u64>();

        assert_eq!(amounts, vec![8, 32, 64, 128, 256, 512]);
        assert_eq!(sum, 1000);
    }
}
