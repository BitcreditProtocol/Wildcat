// ----- standard library imports
// ----- extra library imports
use bcr_common::{client::keys::Client as KeysClient, core_tests};
// ----- local imports

// ----- end imports

#[tokio::test]
async fn list_mintops() {
    let (server, _) = bcr_wdc_core_service::test_utils::build_test_server(None).await;
    let server_url = server.server_address().expect("address");
    let client = KeysClient::new(server_url);

    let kid = core_tests::generate_random_ecash_keyset().0.id;
    let list_mintops = client.list_mint_operations(kid).await.unwrap();
    assert!(list_mintops.is_empty());
}
