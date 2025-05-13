use bcr_wdc_ebill_client::{EbillClient, Error};

#[tokio::test]
async fn contact_calls() {
    let server = bcr_wdc_ebill_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = EbillClient::new(server_url);

    let response = client
        .create_contact(&bcr_wdc_webapi::contact::NewContactPayload {
            t: 0,
            node_id: "some id".into(),
            name: "name".into(),
            email: None,
            postal_address: None,
            date_of_birth_or_registration: None,
            country_of_birth_or_registration: None,
            city_of_birth_or_registration: None,
            identification_number: None,
            avatar_file_upload_id: None,
            proof_document_file_upload_id: None,
        })
        .await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::InvalidRequest));
}
