// ----- standard library imports
// ----- extra library imports
use bcr_common::client::core::Client as CoreClient;
// ----- local imports

#[tokio::test]
async fn list_keys_empty() {
    let (server, _) = bcr_wdc_core_service::test_utils::build_test_server(None).await;
    let server_url = server.server_address().expect("address");
    let client = CoreClient::new(server_url);

    let response = client.list_keys().await.unwrap();
    assert_eq!(response.len(), 0);
}
