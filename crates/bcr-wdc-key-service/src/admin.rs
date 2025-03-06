// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use cashu::nuts::nut00 as cdk00;
// ----- local imports
use crate::error::Result;
use crate::service::{KeysRepository, Service};

#[utoipa::path(
    post,
    path = "/v1/admin/keys/sign/",
    request_body(content = cdk00::BlindedMessage, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = cdk00::BlindedSignature, content_type = "application/json"),
        (status = 404, description = "keyset id not  found"),
    )
)]
pub async fn sign_blind<KR>(
    State(ctrl): State<Service<KR>>,
    Json(blind): Json<cdk00::BlindedMessage>,
) -> Result<Json<cdk00::BlindSignature>>
where
    KR: KeysRepository,
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
pub async fn verify_proof<KR>(
    State(ctrl): State<Service<KR>>,
    Json(proof): Json<cdk00::Proof>,
) -> Result<()>
where
    KR: KeysRepository,
{
    log::debug!("Received verify proof request");
    ctrl.verify_proof(proof).await
}
