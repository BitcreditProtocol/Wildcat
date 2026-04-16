// ----- standard library imports
// ----- extra library imports
use bcr_common::{
    cashu::{nut00 as cdk00, Amount},
    client::{core::Client as CoreClient, mint::Client as MintClient},
    core_tests,
};
use bcr_wdc_utils::keys::test_utils as keys_test;
// ----- local imports

#[tokio::test]
async fn restore() {
    let entry = core_tests::generate_random_ecash_keyset();
    let (server, _) =
        bcr_wdc_core_service::test_utils::build_test_server(Some(entry.clone())).await;
    let server_url = server.server_address().expect("address");
    let corecl = CoreClient::new(server_url.clone());
    let mintcl = MintClient::new(server_url);

    let (msg, _, _) = keys_test::generate_blind(entry.0.id, Amount::from(16));

    corecl
        .sign(&[msg.clone()])
        .await
        .expect("sign blind in prep for test");

    let test_msg = [cdk00::BlindedMessage {
        amount: Amount::ZERO,
        blinded_secret: msg.blinded_secret,
        keyset_id: core_tests::generate_random_ecash_keyset().0.id,
        witness: None,
    }];

    let resp = mintcl.restore(test_msg.to_vec()).await.expect("restore");
    assert_eq!(resp.len(), 1);
    assert_eq!(resp[0].1.amount, msg.amount);
}
