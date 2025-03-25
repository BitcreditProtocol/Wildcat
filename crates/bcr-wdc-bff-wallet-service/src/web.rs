// ----- standard library imports
// ----- extra library imports
use crate::error::Error::CDKClient;
use axum::extract::{Json, Path, State};
use cashu::KeysResponse;
use cashu::nuts::nut01 as cdk01;
use cashu::nuts::nut02 as cdk02;
use cdk::wallet::client::MintConnector;
// ----- local imports
use crate::error::Result;
use crate::service::Service;

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
pub async fn get_mint_keys(State(ctrl): State<Service>) -> Result<Json<cdk01::KeysResponse>> {
    log::debug!("Received /v1/keys request");

    ctrl.get_mint_keys()
        .await
        .map_err(|e| CDKClient(e))
        .map(|it| Json(KeysResponse { keysets: it }))
}

#[utoipa::path(
    get,
    path = "/v1/keys/{kid}",
    params(
        ("kid" = cdk02::Id, Path, description = "The keyset id")
    ),
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
        (status = 404, description = "keyset id not found"),
    )
)]
pub async fn get_mint_keyset(
    State(ctrl): State<Service>,
    Path(kid): Path<cdk02::Id>,
) -> Result<Json<cdk01::KeysResponse>> {
    log::debug!("Received keyset lookup request for id: {}", kid);

    ctrl.get_mint_keyset(kid)
        .await
        .map_err(|e| CDKClient(e))
        .map(|it| {
            Json(KeysResponse {
                keysets: Vec::from([it]),
            })
        })
}
