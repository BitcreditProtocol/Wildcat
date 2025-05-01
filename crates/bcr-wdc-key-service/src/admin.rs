// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_wdc_webapi::keys as web_keys;
use cashu::{nut00 as cdk00, nut02 as cdk02};
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
    ctrl.sign_blind(&blind).await.map(Json)
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
    path = "/v1/admin/keys/generate",
    request_body(content = web_keys::GenerateKeysetRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = cdk02::Id, content_type = "application/json"),
        (status = 404, description = "keyset id not found"),
    )
)]
pub async fn generate<QuotesKeysRepo, KeysRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo>>,
    Json(request): Json<web_keys::GenerateKeysetRequest>,
) -> Result<Json<cdk02::Id>>
where
    QuotesKeysRepo: QuoteKeysRepository,
{
    log::debug!("Received generate request for qid {}", request.qid);
    let kid = ctrl
        .generate_keyset(
            request.qid,
            request.condition.amount,
            request.condition.public_key,
            request.expire,
        )
        .await?;
    Ok(Json(kid))
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
    let sig = ctrl.pre_sign(request.qid, &request.msg).await?;
    Ok(Json(sig))
}

#[utoipa::path(
    post,
    path = "/v1/admin/keys/activate/",
    request_body(content = web_keys::ActivateKeysetRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response"),
        (status = 404, description = "keyset id not found"),
    )
)]
pub async fn activate<QuotesKeysRepo, KeysRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo>>,
    Json(request): Json<web_keys::ActivateKeysetRequest>,
) -> Result<()>
where
    QuotesKeysRepo: QuoteKeysRepository,
    KeysRepo: KeysRepository,
{
    log::debug!("Received activate request for qid {}", request.qid);
    ctrl.activate(&request.qid).await
}
