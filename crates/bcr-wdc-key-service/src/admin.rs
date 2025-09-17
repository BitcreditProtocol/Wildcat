// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_wdc_webapi::keys as web_keys;
// ----- local imports
use crate::error::Result;
use crate::service::Service;

#[utoipa::path(
    post,
    path = "/v1/admin/keys/sign/",
    request_body(content = cashu::BlindedMessage, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = cashu::BlindSignature, content_type = "application/json"),
        (status = 404, description = "keyset id not  found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn sign_blind(
    State(ctrl): State<Service>,
    Json(blind): Json<cashu::BlindedMessage>,
) -> Result<Json<cashu::BlindSignature>> {
    tracing::debug!("Received sign blind request");

    ctrl.sign_blind(&blind).await.map(Json)
}

#[utoipa::path(
    post,
    path = "/v1/admin/keys/verify/",
    request_body(content = cashu::Proof, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = bool, content_type = "application/json"),
        (status = 400, description = "proof verification failed"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn verify_proof(
    State(ctrl): State<Service>,
    Json(proof): Json<cashu::Proof>,
) -> Result<()> {
    tracing::debug!("Received verify proof request");

    ctrl.verify_proof(proof).await
}

#[utoipa::path(
    get,
    path = "/v1/admin/keys/{date}",
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
    State(ctrl): State<Service>,
    Path(date): Path<chrono::NaiveDate>,
) -> Result<Json<cashu::Id>> {
    tracing::debug!("Received get_keyset_for_date request");

    let tstamp = date.and_time(chrono::NaiveTime::default()).and_utc();
    let kid = ctrl.get_keyset_id_for_date(tstamp).await?;
    Ok(Json(kid))
}

#[utoipa::path(
    post,
    path = "/v1/admin/keys/deactivate/",
    request_body(content = web_keys::DeactivateKeysetRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = web_keys::DeactivateKeysetResponse, content_type = "application/json"),
        (status = 404, description = "keyset id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn deactivate(
    State(ctrl): State<Service>,
    Json(request): Json<web_keys::DeactivateKeysetRequest>,
) -> Result<Json<web_keys::DeactivateKeysetResponse>> {
    tracing::debug!("Received deactivate request");

    let kid = ctrl.deactivate(request.kid).await?;
    let response = web_keys::DeactivateKeysetResponse { kid };
    Ok(Json(response))
}
