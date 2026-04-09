// ----- standard library imports
// ----- extra library imports
use bcr_common::{
    core::signature::serialize_n_schnorr_sign_borsh_msg, wire::quotes as wire_quotes,
};
use bcr_wdc_webapi::test_utils::generate_random_bill_enquire_request;
use bdk_wallet::serde_json;
// ----- local imports

// ----- end imports

fn main() -> std::io::Result<()> {
    let owner_key = bcr_common::core_tests::generate_random_keypair();
    let owner_pk = bitcoin::PublicKey::new(owner_key.public_key());
    let (request, signing_key) = generate_random_bill_enquire_request(owner_pk, None, None);
    let (content, signature) = serialize_n_schnorr_sign_borsh_msg(&request, &signing_key).unwrap();
    let signed_request = wire_quotes::SignedEnquireRequest { content, signature };
    let jason = serde_json::to_string_pretty(&signed_request).expect("Failed to serialize request");
    println!("random generated bcr_wdc_webapi::quotes::EnquireRequest in JSON format");
    println!("{jason}");
    Ok(())
}
