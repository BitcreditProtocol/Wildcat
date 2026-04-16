// ----- standard library imports
// ----- extra library imports
use bcr_common::client::mint::Client as MintClient;
// ----- local imports

#[tokio::test]
async fn list_keyset_info_empty() {
    let (server, _) = bcr_wdc_core_service::test_utils::build_test_server(None).await;
    let server_url = server.server_address().expect("address");
    let client = MintClient::new(server_url);
    let response = client.list_keyset_info(Default::default()).await.unwrap();
    assert_eq!(response.len(), 0);
}
