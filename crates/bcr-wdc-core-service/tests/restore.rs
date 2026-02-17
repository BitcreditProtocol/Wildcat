// ----- standard library imports
// ----- extra library imports
use bcr_common::cashu::{nut00 as cdk00, Amount};
use bcr_common::client::keys::Client as KeysClient;
use bcr_wdc_utils::keys::test_utils as keys_test;
// ----- local imports

#[tokio::test]
async fn restore() {
    let entry = keys_test::generate_random_keyset();
    let (server, _) =
        bcr_wdc_core_service::test_utils::build_test_server(Some(entry.clone())).await;
    let server_url = server.server_address().expect("address");
    let client = KeysClient::new(server_url);

    let (msg, _, _) = keys_test::generate_blind(entry.0.id, Amount::from(16));

    client
        .sign(&msg)
        .await
        .expect("sign blind in prep for test");

    let test_msg = [cdk00::BlindedMessage {
        amount: Amount::ZERO,
        blinded_secret: msg.blinded_secret,
        keyset_id: keys_test::generate_random_keysetid(),
        witness: None,
    }];

    let resp = client.restore(test_msg.to_vec()).await.expect("restore");
    assert_eq!(resp.len(), 1);
    assert_eq!(resp[0].1.amount, msg.amount);
}
