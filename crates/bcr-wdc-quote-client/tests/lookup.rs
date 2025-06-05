// ----- standard library imports
// ----- extra library imports
use bcr_wdc_quote_client::{Error, QuoteClient};
use bcr_wdc_utils::keys::test_utils as keys_test;
use bcr_wdc_webapi::test_utils::generate_random_bill_enquire_request;
use uuid::Uuid;
// ----- local imports

// ----- end imports

#[tokio::test]
async fn lookup_id_not_found() {
    let server = bcr_wdc_quote_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = QuoteClient::new(server_url);

    let qid = Uuid::new_v4();
    let response = client.lookup(qid).await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));
}

#[tokio::test]
async fn lookup_id_found() {
    let server = bcr_wdc_quote_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = QuoteClient::new(server_url);

    let (request, signing_key) = generate_random_bill_enquire_request();

    let qid = client
        .enquire(request.content, keys_test::publics()[0], &signing_key)
        .await
        .expect("enquire request");

    let response = client.lookup(qid).await.unwrap();
    assert!(matches!(
        response,
        bcr_wdc_webapi::quotes::StatusReply::Pending
    ))
}
