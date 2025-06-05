// ----- standard library imports
// ----- extra library imports
use bcr_wdc_quote_client::QuoteClient;
use bcr_wdc_utils::keys::test_utils::generate_random_keypair;
use bcr_wdc_webapi::{bill as web_bill, quotes as web_quotes};
// ----- local imports

// ----- end imports

#[tokio::test]
async fn enquire() {
    let server = bcr_wdc_quote_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = QuoteClient::new(server_url);

    let drawer = web_bill::BillIdentParticipant {
        node_id: generate_random_keypair().public_key().to_string(),
        ..Default::default()
    };
    let drawee = web_bill::BillIdentParticipant {
        node_id: generate_random_keypair().public_key().to_string(),
        ..Default::default()
    };
    let payee_keys = generate_random_keypair();
    let payee = web_bill::BillIdentParticipant {
        node_id: payee_keys.public_key().to_string(),
        ..Default::default()
    };
    let bill = web_quotes::BillInfo {
        id: generate_random_keypair().public_key().to_string(),
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
