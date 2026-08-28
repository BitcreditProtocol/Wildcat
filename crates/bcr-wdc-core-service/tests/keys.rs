// ----- standard library imports
// ----- extra library imports
use bcr_common::{client::admin::core::Client as CoreClient, core_tests};
use bcr_wdc_utils::keys as keys_utils;
// ----- local imports

// ----- end imports

#[tokio::test]
async fn keys_simple_request() {
    let (kinfo, keyset) = core_tests::generate_random_ecash_keyset();
    let kentry = keys_utils::to_entry(kinfo, keyset);
    let (server, _) =
        bcr_wdc_core_service::test_utils::build_test_server(Some(kentry.clone())).await;
    let server_url = server.server_address().expect("address");
    let client = CoreClient::new(server_url);

    let keys = client.keys(kentry.id).await.unwrap();
    assert_eq!(keys.id, kentry.id);
}
