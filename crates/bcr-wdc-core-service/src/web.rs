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
use crate::{error::Result, service};

// ----- end imports

/// --------------------------- Look up keysets info
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn lookup_keyset(
    State(ctrl): State<Arc<service::Service>>,
    Path(kid): Path<cashu::Id>,
) -> Result<Json<cashu::KeySetInfo>> {
    tracing::debug!("Received keyset lookup request");

    let info = ctrl.info(kid).await?;
    Ok(Json(info.into()))
}

/// --------------------------- list keysets info
fn convert_keyset_filters(filters: wire_keys::KeysetInfoFilters) -> service::ListFilters {
    service::ListFilters {
        unit: filters.unit,
        min_expiration: filters.min_expiration,
        max_expiration: filters.max_expiration,
    }
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_keysets(
    State(ctrl): State<Arc<service::Service>>,
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
    State(ctrl): State<Arc<service::Service>>,
    Path(kid): Path<cashu::Id>,
) -> Result<Json<cashu::KeysResponse>> {
    let mint_keyset = ctrl.keys(kid).await?.into();
    let keyset = bcr_common::core::keys::to_keyset(&mint_keyset, None);
    let response = cashu::KeysResponse {
        keysets: vec![keyset],
    };
    Ok(Json(response))
}

/// --------------------------- Restore signatures
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn restore(
    State(ctrl): State<Arc<service::Service>>,
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

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, cache))]
pub async fn commit_to_swap(
    State(ctrl): State<Arc<service::Service>>,
    State(cache): State<Arc<dyn nut19::Cache>>,
    Json(request): Json<wire_swap::SwapCommitmentRequest>,
) -> Result<Json<wire_swap::SwapCommitmentResponse>> {
    let now = chrono::Utc::now();
    let key = nut19::swap_commitment::request_to_key(request.clone());
    if let Some(blob) = cache.load(key).await {
        let response = nut19::swap_commitment::blob_to_response(blob);
        return Ok(Json(response));
    }
    let (content, commitment) = ctrl.commit_to_swap(request, now).await?;
    let response = wire_swap::SwapCommitmentResponse {
        content,
        commitment,
    };
    let blob = nut19::swap_commitment::response_to_blob(&response);
    cache.store_and_clean(key, blob, now).await;
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, cache))]
pub async fn swap_tokens(
    State(ctrl): State<Arc<service::Service>>,
    State(cache): State<Arc<dyn nut19::Cache>>,
    Json(request): Json<wire_swap::SwapRequest>,
) -> Result<Json<wire_swap::SwapResponse>> {
    let now = chrono::Utc::now();
    let key = nut19::swap::request_to_key(request.clone());
    if let Some(blob) = cache.load(key).await {
        let response = nut19::swap::blob_to_response(blob);
        return Ok(Json(response));
    }
    let signatures = ctrl
        .swap(request.inputs, request.outputs, request.commitment, now)
        .await?;
    let response = wire_swap::SwapResponse { signatures };
    let blob = nut19::swap::response_to_blob(&response);
    cache.store_and_clean(key, blob, now).await;
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, cache))]
pub async fn signed_commit_to_swap(
    State(ctrl): State<Arc<service::Service>>,
    State(cache): State<Arc<dyn nut19::Cache>>,
    Json(request): Json<wire_swap::SignedSwapCommitmentRequest>,
) -> Result<Json<wire_swap::SwapCommitmentResponse>> {
    let now = chrono::Utc::now();
    let key = nut19::swap_commitment::signed_request_to_key(&request);
    if let Some(blob) = cache.load(key).await {
        let response = nut19::swap_commitment::blob_to_response(blob);
        return Ok(Json(response));
    }
    let (content, commitment) = ctrl
        .signed_commit_to_swap(request.payload, request.signature, now)
        .await?;
    let response = wire_swap::SwapCommitmentResponse {
        content,
        commitment,
    };
    let blob = nut19::swap_commitment::response_to_blob(&response);
    cache.store_and_clean(key, blob, now).await;
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn check_state(
    State(ctrl): State<Arc<service::Service>>,
    Json(request): Json<cashu::CheckStateRequest>,
) -> Result<Json<cashu::CheckStateResponse>> {
    let now = chrono::Utc::now();
    let states = ctrl.check_state(&request.ys, now).await?;
    let response = cashu::CheckStateResponse { states };
    Ok(Json(response))
}
