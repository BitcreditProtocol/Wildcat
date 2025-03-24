// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use cashu::nuts::nut01 as cdk01;
// ----- local imports
use crate::error::Result;
use crate::service::{KeysService, MintService, Service};

#[utoipa::path(
    get,
    path = "/health",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn health() -> Result<&'static str> {
    Ok("{ \"status\": \"OK\" }")
}

/// --------------------------- Look up keys
#[utoipa::path(
    get,
    path = "/v1/keys",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn keys<MS, KS>(State(ctrl): State<Service<MS, KS>>) -> Result<Json<cdk01::KeysResponse>>
where
    MS: MintService,
    KS: KeysService,
{
    log::debug!("Received /v1/keys request");

    let keyset = ctrl.keys().await;
    Ok(keyset?.into())
}
