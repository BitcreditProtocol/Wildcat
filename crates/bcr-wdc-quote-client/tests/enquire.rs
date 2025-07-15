// ----- standard library imports
// ----- extra library imports
use bcr_wdc_quote_client::QuoteClient;
use bcr_wdc_webapi::test_utils::{generate_random_bill_enquire_request, holder_key_pair};
// ----- local imports

// ----- end imports

#[tokio::test]
async fn enquire() {
    let server = bcr_wdc_quote_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = QuoteClient::new(server_url);

    let owner_key = bcr_wdc_utils::keys::test_utils::generate_random_keypair();
    let (request, signing_key) =
        generate_random_bill_enquire_request(owner_key.clone(), Some(holder_key_pair()));
    let _qid = client
        .enquire(request.content, owner_key.public_key().into(), &signing_key)
        .await
        .expect("enquire request");
}
