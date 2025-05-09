// ----- standard library imports
// ----- extra library imports
use bcr_wdc_webapi::quotes::{BillInfo, IdentityPublicData};
use rand::Rng;
// ----- local modules
use bitcoin::secp256k1::Keypair;
use cashu::nuts::nut02 as cdk02;
use cashu::Amount;
// ----- end imports

pub fn random_ebill() -> (Keypair, BillInfo, bitcoin::secp256k1::schnorr::Signature) {
    let bill_id = bcr_wdc_webapi::test_utils::random_bill_id();
    let (_, drawee) = bcr_wdc_webapi::test_utils::random_identity_public_data();
    let (_, drawer) = bcr_wdc_webapi::test_utils::random_identity_public_data();
    let (_, payee) = bcr_wdc_webapi::test_utils::random_identity_public_data();

    let endorsees_size = rand::thread_rng().gen_range(0..3);
    let mut endorsees: Vec<IdentityPublicData> = Vec::with_capacity(endorsees_size);

    let (endorser_kp, endorser) = bcr_wdc_webapi::test_utils::random_identity_public_data();
    endorsees.push(endorser);

    let owner_key = bcr_wdc_utils::keys::test_utils::generate_random_keypair();

    let amount = Amount::from(rand::thread_rng().gen_range(1000..100000));

    let bill = BillInfo {
        id: bill_id,
        maturity_date: random_date(),
        drawee,
        drawer,
        payee,
        endorsees,
        sum: amount.into(),
    };

    let signature = bcr_wdc_utils::keys::schnorr_sign_borsh_msg_with_key(&bill, &endorser_kp)
        .expect("schnorr_sign_borsh_msg_with_key");

    (owner_key, bill, signature)
}

fn random_date() -> String {
    let start = chrono::Utc::now() + chrono::Duration::days(365);
    let days = rand::thread_rng().gen_range(0..365);
    (start + chrono::Duration::days(days)).to_rfc3339()
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
