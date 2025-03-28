// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_wdc_webapi::keys as web_keys;
use cashu::nuts::nut00 as cdk00;
// ----- local imports
use crate::error::Result;
use crate::service::{KeysRepository, QuoteKeysRepository, Service};

#[utoipa::path(
    post,
    path = "/v1/admin/keys/sign/",
    request_body(content = cdk00::BlindedMessage, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = cdk00::BlindSignature, content_type = "application/json"),
        (status = 404, description = "keyset id not  found"),
    )
)]
pub async fn sign_blind<QuotesKeysRepo, KeysRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo>>,
    Json(blind): Json<cdk00::BlindedMessage>,
) -> Result<Json<cdk00::BlindSignature>>
where
    KeysRepo: KeysRepository,
{
    log::debug!("Received sign blind request");
    ctrl.sign_blind(blind).await.map(Json)
}

#[utoipa::path(
    post,
    path = "/v1/admin/keys/verify/",
    request_body(content = cdk00::Proof, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = bool, content_type = "application/json"),
        (status = 400, description = "proof verification failed"),
    )
)]
pub async fn verify_proof<QuotesKeysRepo, KeysRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo>>,
    Json(proof): Json<cdk00::Proof>,
) -> Result<()>
where
    KeysRepo: KeysRepository,
{
    log::debug!("Received verify proof request");
    ctrl.verify_proof(proof).await
}

#[utoipa::path(
    post,
    path = "/v1/admin/keys/pre_sign/",
    request_body(content = web_keys::PreSignRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = cdk00::BlindSignature, content_type = "application/json"),
        (status = 404, description = "keyset id not  found"),
    )
)]
pub async fn pre_sign<QuotesKeysRepo, KeysRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo>>,
    Json(request): Json<web_keys::PreSignRequest>,
) -> Result<Json<cdk00::BlindSignature>>
where
    QuotesKeysRepo: QuoteKeysRepository,
{
    log::debug!("Received pre_sign request");
    let sig = ctrl
        .pre_sign(request.kid, request.qid, request.expire, &request.msg)
        .await?;
    Ok(Json(sig))
}
