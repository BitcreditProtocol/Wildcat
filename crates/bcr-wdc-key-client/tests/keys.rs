// ----- standard library imports
// ----- extra library imports
use bcr_wdc_key_client::{Error, KeyClient};
use bcr_wdc_utils::keys::test_utils as keys_utils;
// ----- local imports

#[tokio::test]
async fn keys_not_found() {
    let server = bcr_wdc_key_service::test_utils::build_test_server(None);
    let server_url = server.server_address().expect("address");
    let client = KeyClient::new(server_url);

    let kid = keys_utils::generate_random_keysetid();
    let response = client.keys(kid).await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));
}
