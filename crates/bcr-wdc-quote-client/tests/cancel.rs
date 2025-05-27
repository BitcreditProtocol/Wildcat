// ----- standard library imports
// ----- extra library imports
use bcr_wdc_quote_client::{Error, QuoteClient};
use uuid::Uuid;
// ----- local imports

#[tokio::test]
async fn cancel_not_found() {
    let server = bcr_wdc_quote_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = QuoteClient::new(server_url);

    let qid = Uuid::new_v4();
    let response = client.cancel(qid).await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));
}
