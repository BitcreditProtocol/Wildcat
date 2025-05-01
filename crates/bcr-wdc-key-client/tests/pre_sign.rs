// ----- standard library imports
// ----- extra library imports
use bcr_wdc_key_client::KeyClient;
use bcr_wdc_utils::keys::test_utils as keys_utils;
use cashu::Amount;
// ----- local imports

#[tokio::test]
async fn pre_sign() {
    let server = bcr_wdc_key_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = KeyClient::new(server_url);

    let qid = uuid::Uuid::new_v4();
    let expiration = chrono::Utc::now() + chrono::Duration::days(2);
    let amount = Amount::from(1000);
    let public_key = keys_utils::publics()[0];

    let kid = client
        .generate(qid, amount, public_key, expiration)
        .await
        .expect("generate call");

    let (blind, ..) = keys_utils::generate_blind(kid, Amount::from(8_u64));
    let signature = client.pre_sign(qid, &blind).await.expect("pre_sign call");

    assert_eq!(signature.keyset_id, kid);
}
