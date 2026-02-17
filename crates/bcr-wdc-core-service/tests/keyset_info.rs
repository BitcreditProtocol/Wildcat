// ----- standard library imports
// ----- extra library imports
use bcr_common::{
    client::keys::{Client as KeysClient, Error as KeysError},
    core_tests,
};
// ----- local imports

#[tokio::test]
async fn keyset_info_not_found() {
    let (server, _) = bcr_wdc_core_service::test_utils::build_test_server(None).await;
    let server_url = server.server_address().expect("address");
    let client = KeysClient::new(server_url);

    let kid = core_tests::generate_random_ecash_keyset().0.id;
    let response = client.keyset_info(kid).await;
    assert!(response.is_err());
    assert!(matches!(
        response.unwrap_err(),
        KeysError::KeysetIdNotFound(_)
    ));
}
