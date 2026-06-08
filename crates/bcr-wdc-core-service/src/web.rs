// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, Path, Query, State};
use bcr_common::{
    cashu,
    wire::{keys as wire_keys, swap as wire_swap},
};
use bcr_wdc_utils::nut19;
// ----- local imports
use crate::{error::Result, keys, swap};

// ----- end imports

/// --------------------------- Look up keysets info
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn lookup_keyset(
    State(ctrl): State<Arc<keys::service::Service>>,
    Path(kid): Path<cashu::Id>,
) -> Result<Json<cashu::KeySetInfo>> {
    tracing::debug!("Received keyset lookup request");

    let info = ctrl.info(kid).await?;
    Ok(Json(info.into()))
}

/// --------------------------- list keysets info
fn convert_keyset_filters(filters: wire_keys::KeysetInfoFilters) -> keys::service::ListFilters {
    keys::service::ListFilters {
        unit: filters.unit,
        min_expiration: filters.min_expiration,
        max_expiration: filters.max_expiration,
    }
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_keysets(
    State(ctrl): State<Arc<keys::service::Service>>,
    Query(filters): Query<wire_keys::KeysetInfoFilters>,
) -> Result<Json<cashu::KeysetResponse>> {
    tracing::debug!("Received keysets list request");

    let list_filters = convert_keyset_filters(filters);
    let infos = ctrl
        .list_info(list_filters)
        .await?
        .into_iter()
        .map(cashu::KeySetInfo::from)
        .collect();
    let response = cashu::KeysetResponse { keysets: infos };
    Ok(Json(response))
}

/// --------------------------- Look up keys
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn lookup_keys(
    State(ctrl): State<Arc<keys::service::Service>>,
    Path(kid): Path<cashu::Id>,
) -> Result<Json<cashu::KeysResponse>> {
    tracing::debug!("Received keyset lookup request: {kid}");

    let keyset = ctrl.keys(kid).await?;
    let response = cashu::KeysResponse {
        keysets: vec![bcr_wdc_utils::keys::to_keyset(&keyset, None)],
    };
    Ok(Json(response))
}

/// --------------------------- Restore signatures
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn restore(
    State(ctrl): State<Arc<keys::service::Service>>,
    Json(req): Json<cashu::RestoreRequest>,
) -> Result<Json<cashu::RestoreResponse>> {
    tracing::debug!("Received wallet restore request");

    let mut response = cashu::RestoreResponse {
        outputs: Vec::new(),
        signatures: Vec::new(),
    };
    for blind in req.outputs.into_iter() {
        let sign_opt = ctrl.search_signature(&blind).await?;
        if let Some(signature) = sign_opt {
            response.signatures.push(signature);
            response.outputs.push(blind);
        }
    }
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(keys, swap, cache))]
pub async fn commit_to_swap(
    State(keys): State<Arc<keys::service::Service>>,
    State(swap): State<Arc<swap::service::Service>>,
    State(cache): State<Arc<dyn nut19::Cache>>,
    Json(request): Json<wire_swap::SwapCommitmentRequest>,
) -> Result<Json<wire_swap::SwapCommitmentResponse>> {
    let now = chrono::Utc::now();
    let key = nut19::swap_commitment::request_to_key(request.clone());
    if let Some(blob) = cache.load(key).await {
        let response = nut19::swap_commitment::blob_to_response(blob);
        return Ok(Json(response));
    }
    let signsrvc = swap::KeysSignService { srvc: keys };
    let (content, commitment) = swap.commit_to_swap(&signsrvc, request, now).await?;
    let response = wire_swap::SwapCommitmentResponse {
        content,
        commitment,
    };
    let blob = nut19::swap_commitment::response_to_blob(&response);
    cache.store_and_clean(key, blob, now).await;
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, keys_srvc, cache))]
pub async fn swap_tokens(
    State(ctrl): State<Arc<swap::service::Service>>,
    State(keys_srvc): State<Arc<keys::service::Service>>,
    State(cache): State<Arc<dyn nut19::Cache>>,
    Json(request): Json<wire_swap::SwapRequest>,
) -> Result<Json<wire_swap::SwapResponse>> {
    let now = chrono::Utc::now();
    let key = nut19::swap::request_to_key(request.clone());
    if let Some(blob) = cache.load(key).await {
        let response = nut19::swap::blob_to_response(blob);
        return Ok(Json(response));
    }
    let signsrvc = swap::KeysSignService { srvc: keys_srvc };
    let signatures = ctrl
        .swap(
            &signsrvc,
            request.inputs,
            request.outputs,
            request.commitment,
            request.attestation,
            now,
        )
        .await?;
    let response = wire_swap::SwapResponse { signatures };
    let blob = nut19::swap::response_to_blob(&response);
    cache.store_and_clean(key, blob, now).await;
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, keys_srvc, cache))]
pub async fn signed_swap_tokens(
    State(ctrl): State<Arc<swap::service::Service>>,
    State(keys_srvc): State<Arc<keys::service::Service>>,
    State(cache): State<Arc<dyn nut19::Cache>>,
    Json(request): Json<wire_swap::SignedSwapRequest>,
) -> Result<Json<wire_swap::SwapResponse>> {
    let now = chrono::Utc::now();
    let key = nut19::signed_swap::request_to_key(&request);
    if let Some(blob) = cache.load(key).await {
        let response = nut19::signed_swap::blob_to_response(blob);
        return Ok(Json(response));
    }
    let signsrvc = swap::KeysSignService { srvc: keys_srvc };
    let signatures = ctrl
        .signed_swap(
            &signsrvc,
            request.content,
            request.signature,
            request.mint_id,
            request.commitment,
            request.attestation,
            now,
        )
        .await?;
    let response = wire_swap::SwapResponse { signatures };
    let blob = nut19::signed_swap::response_to_blob(&response);
    cache.store_and_clean(key, blob, now).await;
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn check_state(
    State(ctrl): State<Arc<swap::service::Service>>,
    Json(request): Json<cashu::CheckStateRequest>,
) -> Result<Json<cashu::CheckStateResponse>> {
    let states = ctrl.check_spendable(&request.ys).await?;
    let response = cashu::CheckStateResponse { states };
    Ok(Json(response))
}
