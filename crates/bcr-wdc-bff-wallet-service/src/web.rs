// ----- extra library imports
use axum::extract::{Json, Path, State};
use cashu::{
    nut01 as cdk01, nut02 as cdk02, nut03 as cdk03, nut04 as cdk04, nut05 as cdk05, nut06 as cdk06,
    nut07 as cdk07, nut09 as cdk09,
};
use cdk::wallet::{HttpClient as CDKClient, MintConnector};
// ----- local imports
use crate::error::{Error, Result};
use crate::AppController;

// ----- end imports

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
pub async fn get_mint_info(State(ctrl): State<CDKClient>) -> Result<Json<cdk06::MintInfo>> {
    log::debug!("Requested /v1/info");

    let info = ctrl.get_mint_info().await?;
    let info = info
        .name("bcr-wdc-bff")
        .description("Bitcredit Wildcat Mint BFF")
        .long_description("Bitcredit Wildcat Mint Backend-For-Frontend");
    Ok(Json(info))
}

#[utoipa::path(
    get,
    path = "/v1/keys",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_mint_keys(State(ctrl): State<AppController>) -> Result<Json<cdk01::KeysResponse>> {
    log::debug!("Requested /v1/keys");

    let mut keys = ctrl.cdk_client.get_mint_keys().await?;
    let mut bcr_keys = ctrl.keys_client.list_keys().await.unwrap_or_default();
    keys.append(&mut bcr_keys);
    let response = cdk01::KeysResponse { keysets: keys };
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/v1/keysets",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_mint_keysets(
    State(ctrl): State<AppController>,
) -> Result<Json<cdk02::KeysetResponse>> {
    log::debug!("Requested /v1/keysets");

    let mut infos = ctrl.cdk_client.get_mint_keysets().await?;
    let mut bcr_infos = ctrl
        .keys_client
        .list_keyset_info()
        .await
        .unwrap_or_default();
    infos.keysets.append(&mut bcr_infos);
    Ok(Json(infos))
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
    State(ctrl): State<AppController>,
    Path(kid): Path<cdk02::Id>,
) -> Result<Json<cdk01::KeysResponse>> {
    log::debug!("Requested /v1/keys/{}", kid);

    let bcr_response = ctrl.keys_client.keys(kid).await;
    if let Ok(keys) = bcr_response {
        let response = cdk01::KeysResponse {
            keysets: vec![keys],
        };
        return Ok(Json(response));
    }
    let keys = ctrl.cdk_client.get_mint_keyset(kid).await?;
    let response = cdk01::KeysResponse {
        keysets: vec![keys],
    };
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/v1/mint/quote/bolt11",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn post_mint_quote(
    State(ctrl): State<CDKClient>,
    Json(request): Json<cdk04::MintQuoteBolt11Request>,
) -> Result<Json<cdk04::MintQuoteBolt11Response<String>>> {
    log::debug!("Requested /v1/mint/quote/bolt11");

    let response = ctrl.post_mint_quote(request).await?;
    Ok(Json(response))
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
    State(ctrl): State<CDKClient>,
    Path(quote_id): Path<String>,
) -> Result<Json<cdk04::MintQuoteBolt11Response<String>>> {
    log::debug!("Requested /v1/mint/quote/bolt11/{}", quote_id);

    let response = ctrl.get_mint_quote_status(quote_id.as_str()).await?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/v1/mint/bolt11",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn post_mint(
    State(ctrl): State<CDKClient>,
    Json(request): Json<cdk04::MintBolt11Request<String>>,
) -> Result<Json<cdk04::MintBolt11Response>> {
    log::debug!("Requested /v1/mint/bolt11");

    let response = ctrl.post_mint(request).await?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/v1/melt/quote/bolt11",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn post_melt_quote(
    State(ctrl): State<CDKClient>,
    Json(request): Json<cdk05::MeltQuoteBolt11Request>,
) -> Result<Json<cdk05::MeltQuoteBolt11Response<String>>> {
    log::debug!("Requested /v1/melt/quote/bolt11");

    let response = ctrl.post_melt_quote(request).await?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/v1/melt/quote/bolt11/{quote_id}",
    params(
        ("quote_id" = &str, Path, description = "The quote id")
    ),
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
        (status = 404, description = "Quote not found"),
    )
)]
pub async fn get_melt_quote_status(
    State(ctrl): State<CDKClient>,
    Path(quote_id): Path<String>,
) -> Result<Json<cdk05::MeltQuoteBolt11Response<String>>> {
    log::debug!("Requested /v1/melt/quote/bolt11/{}", quote_id);

    let response = ctrl.get_melt_quote_status(quote_id.as_str()).await?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/v1/melt/bolt11",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn post_melt(
    State(ctrl): State<CDKClient>,
    Json(request): Json<cdk05::MeltBolt11Request<String>>,
) -> Result<Json<cdk05::MeltQuoteBolt11Response<String>>> {
    log::debug!("Requested /v1/melt/bolt11");

    let response = ctrl.post_melt(request).await?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/v1/swap",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn post_swap(
    State(ctrl): State<AppController>,
    Json(request): Json<cdk03::SwapRequest>,
) -> Result<Json<cdk03::SwapResponse>> {
    log::debug!("Requested /v1/swap");

    // TODO: potential improvement
    // in a separate, testable function
    // - collect keyset IDs from inputs and check if they are from sat vs crsat
    //      if they are mixed, reject the request
    // - collect keyset IDs from outputs and check if they are from sat vs crsat
    //      if they are mixed, reject the request
    // - inputs: crsat -- output: crsat ---> forward to swap_client
    // - inputs: sat -- output: sat ---> forward to cdk_client
    // - inputs: crsat -- output: sat ---> forward to treasury_client
    // - inputs: sat -- output: crsat ---> reject the request
    let bcr_response = ctrl
        .swap_client
        .swap(request.inputs().clone(), request.outputs().clone())
        .await;
    if let Ok(signatures) = bcr_response {
        let response = cdk03::SwapResponse { signatures };
        return Ok(Json(response));
    }

    let redeem_response = ctrl
        .treasury_client
        .redeem(request.inputs().clone(), request.outputs().clone())
        .await;
    if let Ok(signatures) = redeem_response {
        let response = cdk03::SwapResponse { signatures };
        return Ok(Json(response));
    }

    let response = ctrl.cdk_client.post_swap(request).await?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/v1/checkstate",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn post_check_state(
    State(ctrl): State<CDKClient>,
    Json(request): Json<cdk07::CheckStateRequest>,
) -> Result<Json<cdk07::CheckStateResponse>> {
    log::debug!("Requested /v1/checkstate");

    let response = ctrl.post_check_state(request).await?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/v1/restore",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn post_restore(
    State(_ctrl): State<AppController>,
    Json(_request): Json<cdk09::RestoreRequest>,
) -> Result<Json<cdk09::RestoreResponse>> {
    log::debug!("Requested /v1/restore");

    Err(Error::NotYet(String::from("post_restore")))
}
