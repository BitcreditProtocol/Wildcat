// ----- standard library imports
// ----- extra library imports
use bcr_wdc_quote_client::QuoteClient;
use bcr_wdc_webapi::{
    bill as web_bill, quotes as web_quotes,
    test_utils::{random_bill_id, random_identity_public_data},
};
// ----- local imports

// ----- end imports

#[tokio::test]
async fn enquire() {
    let server = bcr_wdc_quote_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = QuoteClient::new(server_url);

    let drawer = random_identity_public_data().1;
    let drawee = random_identity_public_data().1;
    let (payee_keys, payee) = random_identity_public_data();
    let bill = web_quotes::BillInfo {
        id: random_bill_id(),
        drawer,
        drawee,
        payee: web_bill::BillParticipant::Ident(payee),
        maturity_date: String::from("2023-12-01T00:00:00.000Z"),
        sum: 1000,
        endorsees: Vec::new(),
        file_urls: vec![],
    };
    let mint_pubkey = bcr_wdc_utils::keys::test_utils::publics()[0];
    let _result = client
        .enquire(bill, mint_pubkey, &payee_keys)
        .await
        .unwrap();
}
