// ----- standard library imports
// ----- extra library imports
use bcr_wdc_utils::keys::test_utils as keys_test;
use bcr_wdc_webapi::{
    bill::BillParticipant,
    quotes::{BillInfo, EnquireRequest},
};
use bdk_wallet::serde_json;
use cashu::Amount;
use rand::Rng;
// ----- local imports
use bcr_wdc_webapi::test_utils::{random_bill_id, random_date, random_identity_public_data};
// ----- end imports

fn main() -> std::io::Result<()> {
    let bill_id = random_bill_id();
    let (_, drawee) = random_identity_public_data();
    let (_, drawer) = random_identity_public_data();
    let (mut signing_key, payee) = random_identity_public_data();

    let endorsees_size = rand::thread_rng().gen_range(0..3);
    let mut endorsees: Vec<BillParticipant> = Vec::with_capacity(endorsees_size);
    for _ in 0..endorsees_size {
        let (keypair, endorse) = random_identity_public_data();
        endorsees.push(BillParticipant::Ident(endorse));
        signing_key = keypair;
    }

    let public_key = keys_test::publics()[0];
    let amount = Amount::from(rand::thread_rng().gen_range(1000..100000));

    let bill = BillInfo {
        id: bill_id,
        maturity_date: random_date(),
        drawee,
        drawer,
        payee: BillParticipant::Ident(payee),
        endorsees,
        sum: amount.into(),
    };

    let request = EnquireRequest {
        content: bill,
        public_key,
    };
    let signature = bcr_wdc_utils::keys::schnorr_sign_borsh_msg_with_key(&request, &signing_key)
        .expect("schnorr_sign_borsh_msg_with_key");
    let signed_request = bcr_wdc_webapi::quotes::SignedEnquireRequest { request, signature };
    let jason = serde_json::to_string_pretty(&signed_request).expect("Failed to serialize request");
    println!("random generated bcr_wdc_webapi::quotes::EnquireRequest in JSON format");
    println!("{}", jason);
    Ok(())
}
