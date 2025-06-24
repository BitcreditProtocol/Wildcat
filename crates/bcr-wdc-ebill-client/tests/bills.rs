use std::str::FromStr;

use bcr_wdc_ebill_client::{EbillClient, Error};
use bcr_wdc_webapi::bill::BillId;

#[tokio::test]
async fn bill_calls() {
    let server = bcr_wdc_ebill_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = EbillClient::new(server_url);
    let bill_id = BillId::from_str("bitcrt285psGq4Lz4fEQwfM3We5HPznJq8p1YvRaddszFaU5dY").unwrap();

    let response = client.get_bills().await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));

    let response = client.get_bill(&bill_id).await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));

    let response = client.get_bill_endorsements(&bill_id).await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));

    let response = client.get_bill_attachment(&bill_id, "file name").await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));

    let response = client
        .get_bitcoin_private_descriptor_for_bill(&bill_id)
        .await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));

    let response = client
        .request_to_pay_bill(&bcr_wdc_webapi::bill::RequestToPayBitcreditBillPayload {
            bill_id: bill_id.to_owned(),
            currency: "sat".to_string(),
        })
        .await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));
}
