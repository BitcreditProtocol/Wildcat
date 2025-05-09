use std::str::FromStr;

// ----- standard library imports
// ----- extra library imports
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue},
    response::IntoResponse,
    Json,
};
use bcr_ebill_api::{
    data::{self, bill, contact, identity},
    util::{self, file::detect_content_type_for_bytes, ValidationError},
};
use bcr_wdc_webapi::{
    bill::{
        BillCombinedBitcoinKey, BillsResponse, BitcreditBill, RequestToPayBitcreditBillPayload,
    },
    contact::{Contact, ContactType, NewContactPayload},
    identity::{Identity, IdentityType, NewIdentityPayload, SeedPhrase},
};
// ----- local imports

use crate::{error::Result, AppController};
// ----- end imports

#[derive(Debug, Clone, serde::Serialize)]
pub struct SuccessResponse {
    pub success: bool,
}

impl SuccessResponse {
    pub fn new() -> Self {
        Self { success: true }
    }
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_seed_phrase(State(ctrl): State<AppController>) -> Result<Json<SeedPhrase>> {
    tracing::debug!("Received backup seed phrase request");
    let seed_phrase = ctrl.identity_service.get_seedphrase().await?;
    Ok(Json(SeedPhrase {
        seed_phrase: bip39::Mnemonic::from_str(&seed_phrase)
            .map_err(|_| crate::error::Error::InvalidMnemonic)?,
    }))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, payload))]
pub async fn recover_from_seed_phrase(
    State(ctrl): State<AppController>,
    Json(payload): Json<SeedPhrase>,
) -> Result<Json<SuccessResponse>> {
    tracing::debug!("Received restore from seed phrase request");
    ctrl.identity_service
        .recover_from_seedphrase(&payload.seed_phrase.to_string())
        .await?;
    Ok(Json(SuccessResponse::new()))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_identity(State(ctrl): State<AppController>) -> Result<Json<Identity>> {
    tracing::debug!("Received get identity request");
    let my_identity = if !ctrl.identity_service.identity_exists().await {
        return Err(bcr_ebill_api::service::Error::NotFound.into());
    } else {
        let full_identity = ctrl.identity_service.get_full_identity().await?;
        Identity::try_from((full_identity.identity, full_identity.key_pair))
            .map_err(|_| crate::error::Error::IdentityConversion)?
    };
    Ok(Json(my_identity))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, payload))]
pub async fn create_identity(
    State(ctrl): State<AppController>,
    Json(payload): Json<NewIdentityPayload>,
) -> Result<Json<SuccessResponse>> {
    tracing::debug!("Received create identity request");
    if ctrl.identity_service.identity_exists().await {
        return Err(crate::error::Error::IdentityAlreadyExists);
    }

    let current_timestamp = util::date::now().timestamp() as u64;
    ctrl.identity_service
        .create_identity(
            identity::IdentityType::from(IdentityType::try_from(payload.t)?),
            payload.name,
            payload.email,
            data::OptionalPostalAddress::from(payload.postal_address),
            payload.date_of_birth,
            payload.country_of_birth,
            payload.city_of_birth,
            payload.identification_number,
            payload.profile_picture_file_upload_id,
            payload.identity_document_file_upload_id,
            current_timestamp,
        )
        .await?;
    Ok(Json(SuccessResponse::new()))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_bills(
    State(ctrl): State<AppController>,
) -> Result<Json<BillsResponse<BitcreditBill>>> {
    tracing::debug!("Received get bills request");
    let identity = ctrl.identity_service.get_identity().await?;
    let bills = ctrl.bill_service.get_bills(&identity.node_id).await?;
    Ok(Json(BillsResponse {
        bills: bills.into_iter().map(|b| b.into()).collect(),
    }))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_bill_detail(
    State(ctrl): State<AppController>,
    Path(bill_id): Path<String>,
) -> Result<Json<BitcreditBill>> {
    tracing::debug!("Received get bill detail request");
    let current_timestamp = util::date::now().timestamp() as u64;
    let identity = ctrl.identity_service.get_identity().await?;
    let bill_detail = ctrl
        .bill_service
        .get_detail(&bill_id, &identity, &identity.node_id, current_timestamp)
        .await?;
    Ok(Json(bill_detail.into()))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_bill_attachment(
    State(ctrl): State<AppController>,
    Path(bill_id): Path<String>,
    Path(file_name): Path<String>,
) -> Result<impl IntoResponse> {
    tracing::debug!("Received get bill attachment request");
    let keys = ctrl.bill_service.get_bill_keys(&bill_id).await?;
    let file_bytes = ctrl
        .bill_service
        .open_and_decrypt_attached_file(&bill_id, &file_name, &keys.private_key)
        .await
        .map_err(|_| bcr_ebill_api::service::Error::NotFound)?;

    let content_type = detect_content_type_for_bytes(&file_bytes).ok_or(
        bcr_ebill_api::service::Error::Validation(ValidationError::InvalidContentType),
    )?;
    let parsed_content_type: HeaderValue = content_type.parse().map_err(|_| {
        bcr_ebill_api::service::Error::Validation(ValidationError::InvalidContentType)
    })?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, parsed_content_type);

    Ok((headers, file_bytes))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn request_to_pay_bill(
    State(ctrl): State<AppController>,
    Json(request_to_pay_bill_payload): Json<RequestToPayBitcreditBillPayload>,
) -> Result<Json<SuccessResponse>> {
    tracing::debug!("Received request to pay bill request");
    let current_timestamp = util::date::now().timestamp() as u64;
    let identity::IdentityWithAll { identity, key_pair } =
        ctrl.identity_service.get_full_identity().await?;

    ctrl.bill_service
        .execute_bill_action(
            &request_to_pay_bill_payload.bill_id,
            bill::BillAction::RequestToPay(request_to_pay_bill_payload.currency.clone()),
            &contact::BillParticipant::Ident(contact::BillIdentParticipant::new(identity)?),
            &key_pair,
            current_timestamp,
        )
        .await?;

    Ok(Json(SuccessResponse::new()))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn bill_bitcoin_key(
    State(ctrl): State<AppController>,
    Path(bill_id): Path<String>,
) -> Result<Json<BillCombinedBitcoinKey>> {
    tracing::debug!("Received get bill bitcoin private key request");
    let identity::IdentityWithAll { identity, key_pair } =
        ctrl.identity_service.get_full_identity().await?;
    let combined_key = ctrl
        .bill_service
        .get_combined_bitcoin_key_for_bill(
            &bill_id,
            &contact::BillParticipant::Ident(contact::BillIdentParticipant::new(identity)?),
            &key_pair,
        )
        .await?;
    Ok(Json(combined_key.into()))
}

// TODO: remove this, once we don't need to manually add contacts anymore for nostr transport to work
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn create_contact(
    State(ctrl): State<AppController>,
    Json(payload): Json<NewContactPayload>,
) -> Result<Json<Contact>> {
    tracing::debug!("Received create contact request");
    let contact = ctrl
        .contact_service
        .add_contact(
            &payload.node_id,
            contact::ContactType::from(ContactType::try_from(payload.t)?),
            payload.name,
            payload.email,
            payload.postal_address.map(data::PostalAddress::from),
            payload.date_of_birth_or_registration,
            payload.country_of_birth_or_registration,
            payload.city_of_birth_or_registration,
            payload.identification_number,
            payload.avatar_file_upload_id,
            payload.proof_document_file_upload_id,
        )
        .await?;
    Ok(Json(contact.into()))
}
