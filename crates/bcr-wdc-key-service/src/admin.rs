// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_wdc_webapi::keys as web_keys;
use cashu::{nut00 as cdk00, nut02 as cdk02};
// ----- local imports
use crate::error::Result;
use crate::service::{KeysRepository, QuoteKeysRepository, Service, SignaturesRepository};

#[utoipa::path(
    post,
    path = "/v1/admin/keys/sign/",
    request_body(content = cdk00::BlindedMessage, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = cdk00::BlindSignature, content_type = "application/json"),
        (status = 404, description = "keyset id not  found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn sign_blind<QuotesKeysRepo, KeysRepo, SignsRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo, SignsRepo>>,
    Json(blind): Json<cdk00::BlindedMessage>,
) -> Result<Json<cdk00::BlindSignature>>
where
    KeysRepo: KeysRepository,
    SignsRepo: SignaturesRepository,
{
    tracing::debug!("Received sign blind request");
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
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn verify_proof<QuotesKeysRepo, KeysRepo, SignsRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo, SignsRepo>>,
    Json(proof): Json<cdk00::Proof>,
) -> Result<()>
where
    KeysRepo: KeysRepository,
{
    tracing::debug!("Received verify proof request");
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
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn generate<QuotesKeysRepo, KeysRepo, SignsRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo, SignsRepo>>,
    Json(request): Json<web_keys::GenerateKeysetRequest>,
) -> Result<Json<cdk02::Id>>
where
    QuotesKeysRepo: QuoteKeysRepository,
{
    tracing::debug!("Received generate request");
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
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn pre_sign<QuotesKeysRepo, KeysRepo, SignsRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo, SignsRepo>>,
    Json(request): Json<web_keys::PreSignRequest>,
) -> Result<Json<cdk00::BlindSignature>>
where
    QuotesKeysRepo: QuoteKeysRepository,
{
    tracing::debug!("Received pre_sign request");
    let sig = ctrl.pre_sign(request.qid, &request.msg).await?;
    Ok(Json(sig))
}

#[utoipa::path(
    post,
    path = "/v1/admin/keys/enable/",
    request_body(content = web_keys::EnableKeysetRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = web_keys::EnableKeysetResponse, content_type = "application/json"),
        (status = 404, description = "keyset id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn enable<QuotesKeysRepo, KeysRepo, SignsRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo, SignsRepo>>,
    Json(request): Json<web_keys::EnableKeysetRequest>,
) -> Result<Json<web_keys::EnableKeysetResponse>>
where
    QuotesKeysRepo: QuoteKeysRepository,
    KeysRepo: KeysRepository,
{
    tracing::debug!("Received enable request");
    let kid = ctrl.enable(&request.qid).await?;
    let response = web_keys::EnableKeysetResponse { kid };
    Ok(Json(response))
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
pub async fn deactivate<QuotesKeysRepo, KeysRepo, SignsRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo, SignsRepo>>,
    Json(request): Json<web_keys::DeactivateKeysetRequest>,
) -> Result<Json<web_keys::DeactivateKeysetResponse>>
where
    QuotesKeysRepo: QuoteKeysRepository,
    KeysRepo: KeysRepository,
{
    tracing::debug!("Received deactivate request");
    let kid = ctrl.deactivate(request.kid).await?;
    let response = web_keys::DeactivateKeysetResponse { kid };
    Ok(Json(response))
}
