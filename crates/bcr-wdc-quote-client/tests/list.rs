// ----- standard library imports
// ----- extra library imports
use bcr_wdc_quote_client::QuoteClient;
use bcr_wdc_utils::keys::test_utils as keys_test;
use bcr_wdc_webapi::test_utils::generate_random_bill_enquire_request;
// ----- local imports

// ----- end imports

#[tokio::test]
async fn list_all() {
    let server = bcr_wdc_quote_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = QuoteClient::new(server_url);
    let (request, signing_key) = generate_random_bill_enquire_request();
    let qid = client
        .enquire(request.content, keys_test::publics()[0], &signing_key)
        .await
        .expect("enquire request");

    let response = client.list(Default::default()).await.unwrap();
    assert_eq!(response.quotes.len(), 1);
    assert_eq!(response.quotes[0].id, qid);
}

#[tokio::test]
async fn list_filter_bill_id() {
    let server = bcr_wdc_quote_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = QuoteClient::new(server_url);
    let (request, signing_key) = generate_random_bill_enquire_request();
    let ebill_id = request.content.id.clone();
    let qid = client
        .enquire(request.content, keys_test::publics()[0], &signing_key)
        .await
        .expect("enquire request");

    let (request, signing_key) = generate_random_bill_enquire_request();
    client
        .enquire(request.content, keys_test::publics()[1], &signing_key)
        .await
        .expect("enquire request");

    let response = client.list(Default::default()).await.unwrap();
    assert_eq!(response.quotes.len(), 2);

    let filter = bcr_wdc_webapi::quotes::ListParam {
        bill_id: Some(ebill_id),
        ..Default::default()
    };
    let response = client.list(filter).await.unwrap();
    assert_eq!(response.quotes.len(), 1);
    assert_eq!(response.quotes[0].id, qid);
}
