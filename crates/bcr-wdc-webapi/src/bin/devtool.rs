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
    let (request, signing_key) = generate_random_bill_enquire_request();
    let signature = keys_utils::schnorr_sign_borsh_msg_with_key(&request, &signing_key)
        .expect("schnorr_sign_borsh_msg_with_key");
    let signed_request = bcr_wdc_webapi::quotes::SignedEnquireRequest { request, signature };
    let jason = serde_json::to_string_pretty(&signed_request).expect("Failed to serialize request");
    println!("random generated bcr_wdc_webapi::quotes::EnquireRequest in JSON format");
    println!("{}", jason);
    Ok(())
}
