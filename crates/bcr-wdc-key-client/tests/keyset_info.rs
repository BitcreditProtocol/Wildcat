// ----- standard library imports
// ----- extra library imports
use bcr_wdc_key_client::{Error, KeyClient};
use bcr_wdc_utils::keys::test_utils as keys_utils;
// ----- local imports

#[tokio::test]
async fn keyset_info_not_found() {
    let server = bcr_wdc_key_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = KeyClient::new(server_url);

    let kid = keys_utils::generate_random_keysetid();
    let response = client.keyset_info(kid).await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));
}
