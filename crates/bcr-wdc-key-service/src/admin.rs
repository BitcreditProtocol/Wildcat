// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_common::{client::keys::Client, wire::keys as wire_keys};
// ----- local imports
use crate::{error::Result, service};

#[utoipa::path(
    post,
    path = Client::SIGN_EP_V1,
    request_body(content = cashu::BlindedMessage, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = cashu::BlindSignature, content_type = "application/json"),
        (status = 404, description = "keyset id not  found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn sign_blind(
    State(ctrl): State<service::Service>,
    Json(blind): Json<cashu::BlindedMessage>,
) -> Result<Json<cashu::BlindSignature>> {
    tracing::debug!("Received sign blind request");

    ctrl.sign_blind(&blind).await.map(Json)
}

#[utoipa::path(
    post,
    path = Client::VERIFY_PROOF_EP_V1,
    request_body(content = cashu::Proof, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = bool, content_type = "application/json"),
        (status = 400, description = "proof verification failed"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn verify_proof(
    State(ctrl): State<service::Service>,
    Json(proof): Json<cashu::Proof>,
) -> Result<()> {
    tracing::debug!("Received verify proof request");

    ctrl.verify_proof(proof).await
}

#[utoipa::path(
    post,
    path = Client::VERIFY_FINGERPRINT_EP_V1,
    request_body(content = wire_keys::ProofFingerprint, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = bool, content_type = "application/json"),
        (status = 400, description = "proof verification failed"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn verify_fingerprint(
    State(ctrl): State<service::Service>,
    Json(fp): Json<wire_keys::ProofFingerprint>,
) -> Result<()> {
    tracing::debug!("Received verify fingerprint request");

    ctrl.verify_fingerprint(fp.into()).await
}

#[utoipa::path(
    get,
    path = Client::KEYSFOREXPIRATION_EP_V1,
    params(
        ("date" = chrono::NaiveDate, Path, description = "The expiration date")
    ),
    responses (
        (status = 200, description = "Successful response", body = cashu::Id, content_type = "application/json"),
        (status = 404, description = "keyset id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_keyset_for_date(
    State(ctrl): State<service::Service>,
    Path(date): Path<chrono::NaiveDate>,
) -> Result<Json<cashu::Id>> {
    tracing::debug!("Received get_keyset_for_date request");

    let kid = ctrl.get_keyset_id_for_date(date).await?;
    Ok(Json(kid))
}

#[utoipa::path(
    post,
    path = Client::DEACTIVATEKEYSET_EP_V1,
    request_body(content = wire_keys::DeactivateKeysetRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = wire_keys::DeactivateKeysetResponse, content_type = "application/json"),
        (status = 404, description = "keyset id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn deactivate(
    State(ctrl): State<service::Service>,
    Json(request): Json<wire_keys::DeactivateKeysetRequest>,
) -> Result<Json<wire_keys::DeactivateKeysetResponse>> {
    tracing::debug!("Received deactivate request");

    let kid = ctrl.deactivate(request.kid).await?;
    let response = wire_keys::DeactivateKeysetResponse { kid };
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = Client::NEWMINTOP_EP_V1,
    request_body(content = wire_keys::NewMintOperationRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = wire_keys::NewMintOperationResponse, content_type = "application/json"),
        (status = 404, description = "keyset id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn new_mintop(
    State(ctrl): State<service::Service>,
    Json(request): Json<wire_keys::NewMintOperationRequest>,
) -> Result<Json<wire_keys::NewMintOperationResponse>> {
    tracing::debug!("Received new mint operation request");

    ctrl.new_minting_operation(
        request.quote_id,
        request.kid,
        request.pub_key,
        request.target,
    )
    .await?;
    let response = wire_keys::NewMintOperationResponse {};
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = Client::NEWMINTOP_EP_V1,
    params(
        ("qid" = uuid::Uuid, Path, description = "the quote id this minting operation is associated with")
    ),
    responses (
        (status = 200, description = "Successful response", body = cashu::Amount, content_type = "application/json"),
        (status = 404, description = "resource id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn mintop_status(
    State(ctrl): State<service::Service>,
    Path(qid): Path<uuid::Uuid>,
) -> Result<Json<cashu::Amount>> {
    tracing::debug!("Received mint operation status request");

    let amount = ctrl.mintop_status(qid).await?;
    Ok(Json(amount))
}

#[utoipa::path(
    get,
    path = Client::LISTMINTOPS_EP_V1,
    params(
        ("kid" = cashu::Id, Path, description = "the keyset id to list minting operations for")
    ),
    responses (
        (status = 200, description = "Successful response", body = Vec<cashu::Amount>, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_mintops(
    State(ctrl): State<service::Service>,
    Path(kid): Path<cashu::Id>,
) -> Result<Json<Vec<cashu::Amount>>> {
    tracing::debug!("Received list mint operations request");

    let mint_ops = ctrl.list_mintops_for_kid(kid).await?;
    let response = mint_ops.into_iter().map(|mop| mop.minted).collect();
    Ok(Json(response))
}
