// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue},
    response::IntoResponse,
    Json,
};
use bcr_ebill_api::{
    constants::MAX_DOCUMENT_FILE_SIZE_BYTES,
    data::{self, bill, contact, identity},
    util::{self, file::detect_content_type_for_bytes, BcrKeys, ValidationError},
};
use bcr_ebill_core::{
    blockchain::bill::{
        chain::{
            get_bill_parties_from_chain_with_plaintext, get_endorsees_from_chain_with_plaintext,
            BillBlockPlaintextWrapper,
        },
        BillBlock, BillBlockchain,
    },
    SecretKey,
};
use bcr_wdc_webapi::{
    bill::{
        BillCombinedBitcoinKey, BillId, BillPaymentStatus, BillWaitingForPaymentState,
        BillsResponse, BitcreditBill, Endorsement, RequestToPayBitcreditBillPayload,
    },
    identity::{Identity, IdentityType, NewIdentityPayload, SeedPhrase},
    quotes::RequestEncryptedFileUrlPayload,
};
use futures::StreamExt;
use reqwest::StatusCode;
// ----- local imports

use crate::{
    error::{Error, Result},
    AppController,
};
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct SimplifiedBillPaymentStatus {
    payment_status: BillPaymentStatus,
    payment_details: Option<BillWaitingForPaymentState>,
}

// decrypt and validate hashes to get bill chain with plaintext
pub fn get_chain_with_plaintext_from_shared_bill(
    shared_bill: &bcr_wdc_webapi::quotes::SharedBill,
    private_key: &SecretKey,
) -> Result<Vec<BillBlockPlaintextWrapper>> {
    let decoded = util::base58_decode(&shared_bill.data)
        .map_err(|e| Error::SharedBill(format!("base58 decode: {e}")))?;
    let decrypted = util::crypto::decrypt_ecies(&decoded, private_key)
        .map_err(|e| Error::SharedBill(format!("decryption: {e}")))?;

    // check that hash matches
    if shared_bill.hash != util::sha256_hash(&decrypted) {
        return Err(Error::SharedBill("Invalid Hash".to_string()));
    }

    let deserialized: Vec<BillBlockPlaintextWrapper> = borsh::from_slice(&decrypted)
        .map_err(|e| Error::SharedBill(format!("deserialization: {e}")))?;
    Ok(deserialized)
}

/// Validates and decrypts a shared bill.
/// The following checks are made:
/// 1. The receiver needs to be the current E-Bill node
/// 2. Decryption needs to work, and the hash needs to match the unencrypted data
/// 3. A valid Bill chain can be built from the data
/// 4. The plaintext hashes of the blocks match the plaintext
/// 5. The signature needs to match
/// 6. All shared files need to match the file hashes
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, payload))]
pub async fn validate_and_decrypt_shared_bill(
    State(ctrl): State<AppController>,
    Json(payload): Json<bcr_wdc_webapi::quotes::SharedBill>,
) -> Result<Json<bcr_wdc_webapi::quotes::BillInfo>> {
    tracing::debug!("Received validate and decrypt shared bill request");
    let identity::IdentityWithAll { identity, key_pair } =
        ctrl.identity_service.get_full_identity().await?;

    // check that our pub key is the receiver pub key
    if identity.node_id.pub_key() != payload.receiver {
        return Err(Error::SharedBill("Public keys don't match".into()));
    }

    // decrypt data
    let chain_with_plaintext =
        get_chain_with_plaintext_from_shared_bill(&payload, &key_pair.get_private_key())
            .map_err(|e| Error::SharedBill(e.to_string()))?;

    // validate chain
    BillBlockchain::new_from_blocks(
        chain_with_plaintext
            .iter()
            .map(|wrapper| wrapper.block.to_owned())
            .collect::<Vec<BillBlock>>(),
    )
    .map_err(|e| Error::SharedBill(format!("invalid chain: {e}")))?;

    // validate plaintext hash
    for block_wrapper in chain_with_plaintext.iter() {
        if block_wrapper.block.plaintext_hash
            != util::sha256_hash(&block_wrapper.plaintext_data_bytes)
        {
            return Err(Error::SharedBill("Plaintext hash mismatch".into()));
        }
    }

    // get data
    let bill_data = match chain_with_plaintext.first() {
        Some(issue_block) => issue_block
            .get_bill_data()
            .map_err(|e| Error::SharedBill(e.to_string()))?,
        None => {
            return Err(Error::SharedBill("Empty chain".into()));
        }
    };

    // get participants
    let bill_parties = get_bill_parties_from_chain_with_plaintext(&chain_with_plaintext)
        .map_err(|e| Error::SharedBill(e.to_string()))?;
    let endorsees = get_endorsees_from_chain_with_plaintext(&chain_with_plaintext);
    let holder = bill_parties.endorsee.unwrap_or(bill_parties.payee.clone());

    // verify signature
    match util::crypto::verify(
        &payload.hash,
        &payload.signature,
        &holder.node_id().pub_key(),
    ) {
        Ok(res) => {
            if !res {
                return Err(Error::SharedBill("Invalid signature".into()));
            }
        }
        Err(e) => return Err(Error::SharedBill(e.to_string())),
    };

    // validate files by downloading, encrypting and checking hashes
    if !payload.file_urls.is_empty() {
        let bill_file_hashes: Vec<String> =
            bill_data.files.iter().map(|f| f.hash.clone()).collect();
        let mut file_hashes = Vec::with_capacity(bill_file_hashes.len());
        for file_url in payload.file_urls.iter() {
            let (_, decrypted) =
                do_get_encrypted_bill_file_from_request_to_mint(&key_pair, file_url).await?;
            file_hashes.push(util::sha256_hash(&decrypted));
        }
        // all of the shared file hashes have to be present on the bill
        if file_hashes.len() != bill_file_hashes.len()
            || !file_hashes.iter().all(|f| bill_file_hashes.contains(f))
        {
            return Err(Error::SharedBill("File hashes don't match".into()));
        }
    }

    let core_drawer: bcr_ebill_core::contact::BillIdentParticipant = bill_parties.drawer.into();
    let core_drawee: bcr_ebill_core::contact::BillIdentParticipant = bill_parties.drawee.into();
    let core_payee: bcr_ebill_core::contact::BillParticipant = bill_parties.payee.into();
    let core_endorsees: Vec<bcr_wdc_webapi::bill::BillParticipant> =
        endorsees.into_iter().map(|e| e.into()).collect();

    // create result
    Ok(Json(bcr_wdc_webapi::quotes::BillInfo {
        id: bill_data.id,
        drawee: core_drawee.into(),
        drawer: core_drawer.into(),
        payee: core_payee.into(),
        endorsees: core_endorsees,
        sum: bill_data.sum,
        maturity_date: util::date::date_string_to_rfc3339(&bill_data.maturity_date)
            .map_err(|e| Error::SharedBill(format!("invalid date format: {e}")))?,
        file_urls: payload.file_urls,
    }))
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
        Identity::try_from(full_identity.identity)
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
    Path(bill_id): Path<BillId>,
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
pub async fn get_bill_payment_status(
    State(ctrl): State<AppController>,
    Path(bill_id): Path<BillId>,
) -> Result<Json<SimplifiedBillPaymentStatus>> {
    tracing::debug!("Received get bill payment status request");
    let current_timestamp = util::date::now().timestamp() as u64;
    let identity = ctrl.identity_service.get_identity().await?;
    let bill_detail = ctrl
        .bill_service
        .get_detail(&bill_id, &identity, &identity.node_id, current_timestamp)
        .await?;
    let payment_status = bill_detail.status.payment;
    let payment_details = match bill_detail.current_waiting_state {
        Some(bcr_ebill_api::data::bill::BillCurrentWaitingState::Payment(payment)) => {
            Some(payment.into())
        }
        _ => None,
    };

    Ok(Json(SimplifiedBillPaymentStatus {
        payment_status: payment_status.into(),
        payment_details,
    }))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_bill_endorsements(
    State(ctrl): State<AppController>,
    Path(bill_id): Path<BillId>,
) -> Result<Json<Vec<Endorsement>>> {
    tracing::debug!("Received get bill detail request");
    let identity = ctrl.identity_service.get_identity().await?;
    let endorsements = ctrl
        .bill_service
        .get_endorsements(&bill_id, &identity.node_id)
        .await?;
    Ok(Json(endorsements.into_iter().map(|e| e.into()).collect()))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_bill_attachment(
    State(ctrl): State<AppController>,
    Path((bill_id, file_name)): Path<(BillId, String)>,
) -> Result<impl IntoResponse> {
    tracing::debug!("Received get bill attachment request");
    let current_timestamp = util::date::now().timestamp() as u64;
    let identity = ctrl.identity_service.get_identity().await?;
    // get bill
    let bill = ctrl
        .bill_service
        .get_detail(&bill_id, &identity, &identity.node_id, current_timestamp)
        .await?;

    // check if this file even exists on the bill
    let file = match bill.data.files.iter().find(|f| f.name == file_name) {
        Some(f) => f,
        None => {
            return Err(bcr_ebill_api::service::bill_service::Error::NotFound.into());
        }
    };

    let keys = ctrl.bill_service.get_bill_keys(&bill_id).await?;
    let file_bytes = ctrl
        .bill_service
        .open_and_decrypt_attached_file(&bill_id, file, &keys.private_key)
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

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, bill_file_url_req))]
pub async fn get_encrypted_bill_file_from_request_to_mint(
    State(ctrl): State<AppController>,
    Query(bill_file_url_req): Query<RequestEncryptedFileUrlPayload>,
) -> Result<impl IntoResponse> {
    tracing::debug!(
        "Received get encrypted bill file from request to mint, url: {}",
        bill_file_url_req.file_url
    );

    let keys = ctrl.identity_service.get_full_identity().await?.key_pair;
    let (content_type, decrypted) =
        do_get_encrypted_bill_file_from_request_to_mint(&keys, &bill_file_url_req.file_url).await?;
    let parsed_content_type: HeaderValue = content_type.parse().map_err(|_| {
        bcr_ebill_api::service::Error::Validation(ValidationError::InvalidContentType)
    })?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, parsed_content_type);

    Ok((headers, decrypted))
}

async fn do_get_encrypted_bill_file_from_request_to_mint(
    keys: &BcrKeys,
    file_url: &url::Url,
) -> Result<(String, Vec<u8>)> {
    if file_url.scheme() != "https" {
        return Err(Error::FileDownload("Only HTTPS urls are allowed".into()));
    }

    // fetch the file by URL
    let resp = reqwest::get(file_url.clone()).await.map_err(|e| {
        tracing::error!("Error downloading file from {}: {e}", file_url.to_string());
        Error::FileDownload("Could not download file".into())
    })?;

    // check status code
    if resp.status() != StatusCode::OK {
        return Err(Error::FileDownload("Could not download file".into()));
    }

    // check content length
    match resp.content_length() {
        Some(len) => {
            if len > MAX_DOCUMENT_FILE_SIZE_BYTES as u64 {
                return Err(Error::FileDownload("File too large".into()));
            }
        }
        None => {
            return Err(Error::FileDownload("no Content-Length set".into()));
        }
    };
    // stream bytes and stop if response gets too large
    let mut stream = resp.bytes_stream();
    let mut total: usize = 0;
    let mut file_bytes = Vec::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            tracing::error!("Error downloading file from {}: {e}", file_url.to_string());
            Error::FileDownload("Could not download file".into())
        })?;
        total += chunk.len();
        if total > MAX_DOCUMENT_FILE_SIZE_BYTES {
            return Err(Error::FileDownload("File too large".into()));
        }
        file_bytes.extend_from_slice(&chunk);
    }

    // decrypt file with private key
    let decrypted =
        util::crypto::decrypt_ecies(&file_bytes, &keys.get_private_key()).map_err(|e| {
            tracing::error!("Error decrypting file from {}: {e}", file_url.to_string());
            Error::FileDownload("Decryption Error".into())
        })?;

    // detect content type and return response
    let content_type = detect_content_type_for_bytes(&decrypted).ok_or(
        bcr_ebill_api::service::Error::Validation(ValidationError::InvalidContentType),
    )?;

    Ok((content_type, decrypted))
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
    Path(bill_id): Path<BillId>,
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
