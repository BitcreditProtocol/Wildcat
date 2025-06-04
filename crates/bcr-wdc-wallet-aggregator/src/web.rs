// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_wdc_key_client::KeyClient;
use cashu::{
    nut00 as cdk00, nut01 as cdk01, nut02 as cdk02, nut03 as cdk03, nut04 as cdk04, nut05 as cdk05,
    nut06 as cdk06, nut07 as cdk07, nut09 as cdk09,
};
use cdk::wallet::{HttpClient as CDKClient, MintConnector};
use futures::future::JoinAll;
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
    tracing::debug!("Requested /v1/info");

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
    tracing::debug!("Requested /v1/keys");

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
    tracing::debug!("Requested /v1/keysets");

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
    tracing::debug!("Requested /v1/keys/{}", kid);

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
    tracing::debug!("Requested /v1/mint/quote/bolt11");

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
    tracing::debug!("Requested /v1/mint/quote/bolt11/{}", quote_id);

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
    tracing::debug!("Requested /v1/mint/bolt11");

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
    tracing::debug!("Requested /v1/melt/quote/bolt11");

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
    tracing::debug!("Requested /v1/melt/quote/bolt11/{}", quote_id);

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
    tracing::debug!("Requested /v1/melt/bolt11");

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
    tracing::debug!("Requested /v1/swap");

    let swap_type = determine_swap_type(
        &ctrl.keys_client,
        request.inputs().as_slice(),
        request.outputs().as_slice(),
    )
    .await?;
    let signatures = match swap_type {
        SwapType::CrSat2CrSat => {
            ctrl.swap_client
                .swap(request.inputs().to_vec(), request.outputs().to_vec())
                .await?
        }
        SwapType::CrSat2Sat => {
            ctrl.treasury_client
                .redeem(request.inputs().to_vec(), request.outputs().to_vec())
                .await?
        }
        SwapType::Sat2Sat => ctrl
            .cdk_client
            .post_swap(request)
            .await
            .map(|resp| resp.signatures)?,
    };

    let response = cdk03::SwapResponse { signatures };
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
    State(ctrl): State<AppController>,
    Json(request): Json<cdk07::CheckStateRequest>,
) -> Result<Json<cdk07::CheckStateResponse>> {
    tracing::info!("Requested /v1/checkstate");

    let n = request.ys.len();
    let credit_states = ctrl.swap_client.check_state(request.ys.clone()).await?;
    assert_eq!(credit_states.len(), n);
    let debit_states = ctrl.cdk_client.post_check_state(request).await?.states;
    assert_eq!(debit_states.len(), n);

    tracing::info!("Debit states: {:?}", debit_states);
    tracing::info!("Credit states: {:?}", credit_states);

    let mut merged = Vec::new();
    for (debit, credit) in debit_states.iter().zip(credit_states.iter()) {
        if debit.state != cashu::nut07::State::Unspent
            && credit.state != cashu::nut07::State::Unspent
        {
            // This should not happen
            panic!("Coin spent both on debit and credit");
        }
        if debit.state != cashu::nut07::State::Unspent {
            merged.push(debit.clone());
        } else {
            merged.push(credit.clone());
        }
    }
    Ok(Json(cdk07::CheckStateResponse { states: merged }))
}

#[utoipa::path(
    post,
    path = "/v1/restore",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn post_restore(
    State(ctrl): State<AppController>,
    Json(request): Json<cdk09::RestoreRequest>,
) -> Result<Json<cdk09::RestoreResponse>> {
    tracing::debug!("Requested /v1/restore");

    let outputs = request.outputs.clone();
    let crsat_signatures = ctrl.keys_client.restore(outputs.clone()).await?;
    let restore_resp = ctrl.cdk_client.post_restore(request).await?;
    let sat_signatures = restore_resp
        .outputs
        .into_iter()
        .zip(restore_resp.signatures.into_iter())
        .collect::<Vec<_>>();

    let mut response = cdk09::RestoreResponse {
        outputs: Default::default(),
        signatures: Default::default(),
        promises: Default::default(),
    };
    // we assume that both sat_signatures and crsat_signatures are ordered
    // according to the order of request.outputs
    // as described in NUT09
    let mut crsat_c = 0;
    let mut sat_c = 0;
    for blind in outputs {
        if let Some(element) = crsat_signatures.get(crsat_c) {
            if blind.blinded_secret == element.0.blinded_secret {
                response.outputs.push(element.0.clone());
                response.signatures.push(element.1.clone());
                crsat_c += 1;
            }
        }
        if let Some(element) = sat_signatures.get(sat_c) {
            if blind.blinded_secret == element.0.blinded_secret {
                response.outputs.push(element.0.clone());
                response.signatures.push(element.1.clone());
                sat_c += 1;
            }
        }
    }
    Ok(Json(response))
}

#[allow(clippy::enum_variant_names)]
enum SwapType {
    CrSat2CrSat,
    Sat2Sat,
    CrSat2Sat,
}

/// if any keyset ID among the inputs is not found in crsat-key-service, then the swap can only be
/// a sat2sat
/// once proved that all inputs are found in crsat-key-service,
/// if any keyset ID among the outputs is not found in crsat-key-service, then the swap can only be
/// a crsat2sat
/// once proved that all outputs are found in crsat-key-service,
/// then it's definitely a crsat2crsat swap
/// it's not a responsibility of this service to deal with the case of mixed inputs/outputs
async fn determine_swap_type(
    key_cl: &KeyClient,
    inputs: &[cdk00::Proof],
    outputs: &[cdk00::BlindedMessage],
) -> Result<SwapType> {
    let inputs_requests: JoinAll<_> = inputs
        .iter()
        .map(|proof| key_cl.keyset_info(proof.keyset_id))
        .collect();
    for response in inputs_requests.await.into_iter() {
        match response {
            Err(bcr_wdc_key_client::Error::ResourceNotFound(_)) => return Ok(SwapType::Sat2Sat),
            Err(e) => return Err(Error::Keys(e)),
            Ok(_) => {}
        }
    }

    let outputs_requests: JoinAll<_> = outputs
        .iter()
        .map(|blind| key_cl.keyset_info(blind.keyset_id))
        .collect();
    for response in outputs_requests.await.into_iter() {
        match response {
            Err(bcr_wdc_key_client::Error::ResourceNotFound(_)) => return Ok(SwapType::CrSat2Sat),
            Err(e) => return Err(Error::Keys(e)),
            Ok(_) => {}
        }
    }
    Ok(SwapType::CrSat2CrSat)
}
