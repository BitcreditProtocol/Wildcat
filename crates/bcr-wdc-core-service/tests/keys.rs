// ----- standard library imports
// ----- extra library imports
use bcr_common::{
    client::admin::core::{Client as CoreClient, Error as CoreError},
    core_tests,
};
// ----- local imports

// ----- end imports

#[tokio::test]
async fn keys_not_found() {
    let (server, _) = bcr_wdc_core_service::test_utils::build_test_server(None).await;
    let server_url = server.server_address().expect("address");
    let client = CoreClient::new(server_url);

    let kid = core_tests::generate_random_ecash_keyset().0.id;
    let response = client.keys(kid).await;
    assert!(response.is_err());
    assert!(matches!(
        response.unwrap_err(),
        CoreError::ResourceNotFound(_)
    ));
}
