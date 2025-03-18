// ----- standard library imports
// ----- extra library imports
use bcr_wdc_key_client::KeyClient;
use bcr_wdc_keys::test_utils as key_utils;
use cashu::Amount;
// ----- local imports

#[tokio::test]
async fn pre_sign() {
    let server = bcr_wdc_key_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = KeyClient::new(server_url).expect("client");

    let kid = key_utils::generate_random_keysetid().into();
    let qid = uuid::Uuid::new_v4();
    let expiration = chrono::Utc::now() + chrono::Duration::days(2);
    let (blind, ..) = key_utils::generate_blind(kid, Amount::from(8_u64));
    let signature = client
        .pre_sign(kid, qid, expiration, &blind)
        .await
        .expect("pre_sign call");
    assert_eq!(kid, signature.keyset_id);
}
