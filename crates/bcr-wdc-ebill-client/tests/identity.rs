use bcr_wdc_ebill_client::{EbillClient, Error};

#[tokio::test]
async fn identity_calls() {
    let server = bcr_wdc_ebill_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = EbillClient::new(server_url);

    let response = client.backup_seed_phrase().await;
    assert!(response.is_ok());

    let response = client
        .restore_from_seed_phrase(&bcr_wdc_webapi::identity::SeedPhrase {
            seed_phrase: bip39::Mnemonic::generate(12).unwrap(),
        })
        .await;
    assert!(response.is_ok());

    let response = client.get_identity().await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::ResourceNotFound(_)));

    let response = client
        .create_identity(&bcr_wdc_webapi::identity::NewIdentityPayload {
            t: 0,
            name: "name".into(),
            email: None,
            postal_address: bcr_wdc_webapi::identity::OptionalPostalAddress {
                country: None,
                city: None,
                zip: None,
                address: None,
            },
            date_of_birth: None,
            country_of_birth: None,
            city_of_birth: None,
            identification_number: None,
            profile_picture_file_upload_id: None,
            identity_document_file_upload_id: None,
        })
        .await;
    assert!(response.is_err());
    assert!(matches!(response.unwrap_err(), Error::InvalidRequest));
}
