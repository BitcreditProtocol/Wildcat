// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_common::{cashu, wire::swap as wire_swap};
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
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_keysets(
    State(ctrl): State<Arc<keys::service::Service>>,
) -> Result<Json<cashu::KeysetResponse>> {
    tracing::debug!("Received keysets list request");

    let infos = ctrl
        .list_info()
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
        keysets: vec![keyset.into()],
    };
    Ok(Json(response))
}

/// --------------------------- List keys
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_keys(
    State(ctrl): State<Arc<keys::service::Service>>,
) -> Result<Json<cashu::KeysResponse>> {
    tracing::debug!("Received keys list request");

    let keysets = ctrl
        .list_keyset()
        .await?
        .into_iter()
        .map(cashu::KeySet::from)
        .collect();
    let response = cashu::KeysResponse { keysets };
    Ok(Json(response))
}

/// --------------------------- Mint
pub async fn mint_ebill(
    State(ctrl): State<Arc<keys::service::Service>>,
    Json(req): Json<cashu::MintRequest<uuid::Uuid>>,
) -> Result<Json<cashu::MintResponse>> {
    tracing::debug!("Received mint request for {}", req.quote);

    let response = ctrl.mint(&req).await?;
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
        promises: None,
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

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, sign_service))]
pub async fn swap_tokens(
    State(ctrl): State<Arc<swap::service::Service>>,
    State(sign_service): State<Arc<keys::service::Service>>,
    Json(request): Json<cashu::SwapRequest>,
) -> Result<Json<cashu::SwapResponse>> {
    let signatures = ctrl
        .swap(sign_service.as_ref(), request.inputs(), request.outputs())
        .await?;
    let response = cashu::SwapResponse { signatures };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, sign_service))]
pub async fn burn_tokens(
    State(ctrl): State<Arc<swap::service::Service>>,
    State(sign_service): State<Arc<keys::service::Service>>,
    Json(request): Json<wire_swap::BurnRequest>,
) -> Result<Json<wire_swap::BurnResponse>> {
    let ys = ctrl.burn(sign_service.as_ref(), &request.proofs).await?;
    Ok(Json(wire_swap::BurnResponse { ys }))
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
