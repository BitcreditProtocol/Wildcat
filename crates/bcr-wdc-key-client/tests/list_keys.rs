// ----- standard library imports
// ----- extra library imports
use bcr_wdc_key_client::KeyClient;
// ----- local imports

#[tokio::test]
async fn list_keys_empty() {
    let server = bcr_wdc_key_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = KeyClient::new(server_url);

    let response = client.list_keys().await.unwrap();
    assert_eq!(response.len(), 0);
}
