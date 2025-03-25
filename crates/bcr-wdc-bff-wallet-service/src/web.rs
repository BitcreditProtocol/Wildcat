// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
use cashu::KeysResponse;
use cashu::nuts::nut01 as cdk01;
use cashu::nuts::nut02 as cdk02;
use cashu::nuts::nut04 as cdk04;
use cashu::nuts::nut06 as cdk06;
use cdk::wallet::client::MintConnector;
// ----- local imports
use crate::error::Error::CDKClient;
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

#[utoipa::path(
    get,
    path = "/v1/info",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_mint_info(State(ctrl): State<Service>) -> Result<Json<cdk06::MintInfo>> {
    log::debug!("Requested /v1/info");

    ctrl.get_mint_info()
        .await
        .map_err(|e| CDKClient(e))
        .map(|it| {
            Json(
                it.name("bcr-wdc-bff-wallet")
                    .description("Bitcredit Wildcat Mint BFF")
                    .long_description("Bitcredit Wildcat Mint Backend-For-Frontend"),
            )
        })
}

#[utoipa::path(
    get,
    path = "/v1/keys",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_mint_keys(State(ctrl): State<Service>) -> Result<Json<cdk01::KeysResponse>> {
    log::debug!("Requested /v1/keys");

    ctrl.get_mint_keys()
        .await
        .map_err(|e| CDKClient(e))
        .map(|it| Json(KeysResponse { keysets: it }))
}

#[utoipa::path(
    get,
    path = "/v1/keysets",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_mint_keysets(State(ctrl): State<Service>) -> Result<Json<cdk02::KeysetResponse>> {
    log::debug!("Requested /v1/keysets");

    ctrl.get_mint_keysets()
        .await
        .map_err(|e| CDKClient(e))
        .map(|it| Json(it))
}

#[utoipa::path(
    get,
    path = "/v1/keys/{kid}",
    params(
        ("kid" = cdk02::Id, Path, description = "The keyset id")
    ),
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
        (status = 404, description = "Keyset not found"),
    )
)]
pub async fn get_mint_keyset(
    State(ctrl): State<Service>,
    Path(kid): Path<cdk02::Id>,
) -> Result<Json<cdk01::KeysResponse>> {
    log::debug!("Requested /v1/keys/{}", kid);

    ctrl.get_mint_keyset(kid)
        .await
        .map_err(|e| CDKClient(e))
        .map(|it| {
            Json(KeysResponse {
                keysets: Vec::from([it]),
            })
        })
}

#[utoipa::path(
    post,
    path = "/v1/mint/quote/bolt11",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn post_mint_quote(
    State(ctrl): State<Service>,
    Json(request): Json<cdk04::MintQuoteBolt11Request>,
) -> Result<Json<cdk04::MintQuoteBolt11Response<String>>> {
    log::debug!("Requested /v1/mint/quote/bolt11");

    ctrl.post_mint_quote(request)
        .await
        .map_err(|e| CDKClient(e))
        .map(|it| Json(it))
}

#[utoipa::path(
    get,
    path = "/v1/mint/quote/bolt11/{quote_id}",
    params(
        ("quote_id" = &str, Path, description = "The quote id")
    ),
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
        (status = 404, description = "Quote not found"),
    )
)]
pub async fn get_mint_quote_status(
    State(ctrl): State<Service>,
    Path(quote_id): Path<String>,
) -> Result<Json<cdk04::MintQuoteBolt11Response<String>>> {
    log::debug!("Requested /v1/mint/quote/bolt11/{}", quote_id);

    ctrl.get_mint_quote_status(quote_id.as_str())
        .await
        .map_err(|e| CDKClient(e))
        .map(|it| Json(it))
}

#[utoipa::path(
    post,
    path = "/v1/mint/bolt11",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn post_mint(
    State(ctrl): State<Service>,
    Json(request): Json<cdk04::MintBolt11Request<String>>,
) -> Result<Json<cdk04::MintBolt11Response>> {
    log::debug!("Requested /v1/mint/bolt11");

    ctrl.post_mint(request)
        .await
        .map_err(|e| CDKClient(e))
        .map(|it| Json(it))
}
