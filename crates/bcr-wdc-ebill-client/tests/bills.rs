use bcr_wdc_ebill_client::{EbillClient, Error};

#[tokio::test]
async fn bill_calls() {
    let server = bcr_wdc_ebill_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = EbillClient::new(server_url);

    let response = client.get_bills().await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));

    let response = client.get_bill("some_id").await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));

    let response = client.get_bill_endorsements("some_id").await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));

    let response = client.get_bill_attachment("some_id", "file name").await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));

    let response = client.get_bitcoin_private_key_for_bill("some_id").await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));

    let response = client
        .request_to_pay_bill(&bcr_wdc_webapi::bill::RequestToPayBitcreditBillPayload {
            bill_id: "some_id".to_string(),
            currency: "sat".to_string(),
        })
        .await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));
}
