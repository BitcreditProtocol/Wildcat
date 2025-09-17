// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
// ----- local imports
use crate::error::Result;
use crate::service::Service;

// ----- end imports

/// --------------------------- Look up keysets info
#[utoipa::path(
    get,
    path = "/v1/keysets/{kid}",
    params(
        ("kid" = cashu::Id, Path, description = "The keyset id")
    ),
    responses (
        (status = 200, description = "Successful response", body = cashu::KeySetInfo, content_type = "application/json"),
        (status = 404, description = "keyset id not  found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn lookup_keyset(
    State(ctrl): State<Service>,
    Path(kid): Path<cashu::Id>,
) -> Result<Json<cashu::KeySetInfo>> {
    tracing::debug!("Received keyset lookup request");

    let info = ctrl.info(kid).await?;
    Ok(Json(info.into()))
}

/// --------------------------- list keysets info
#[utoipa::path(
    get,
    path = "/v1/keysets",
    params(),
    responses (
        (status = 200, description = "Successful response", body = cashu::KeysetResponse, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_keysets(State(ctrl): State<Service>) -> Result<Json<cashu::KeysetResponse>> {
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
#[utoipa::path(
    get,
    path = "/v1/keys/{kid}",
    params(
        ("kid" = cashu::Id, Path, description = "The keyset id")
    ),
    responses (
        (status = 200, description = "Successful response", body = cashu::KeySet, content_type = "application/json"),
        (status = 404, description = "keyset id not  found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn lookup_keys(
    State(ctrl): State<Service>,
    Path(kid): Path<cashu::Id>,
) -> Result<Json<cashu::KeySet>> {
    tracing::debug!("Received keyset lookup request");

    let keyset = ctrl.keys(kid).await?;
    Ok(Json(keyset.into()))
}

/// --------------------------- List keys
#[utoipa::path(
    get,
    path = "/v1/keys",
    params(),
    responses (
        (status = 200, description = "Successful response", body = cashu::KeysResponse, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_keys(State(ctrl): State<Service>) -> Result<Json<cashu::KeysResponse>> {
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
#[utoipa::path(
    post,
    path = "/v1/mint/ebill",
    request_body(content = cashu::MintRequest<String>, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = cashu::MintResponse, content_type = "application/json"),
    )
)]
pub async fn mint_ebill(
    State(ctrl): State<Service>,
    Json(req): Json<cashu::MintRequest<uuid::Uuid>>,
) -> Result<Json<cashu::MintResponse>> {
    tracing::debug!("Received mint request for");

    let response = ctrl.mint(&req).await?;
    Ok(Json(response))
}

/// --------------------------- Restore signatures
#[utoipa::path(
    post,
    path = "/v1/restore",
    request_body(content = cashu::RestoreRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = cashu::RestoreResponse, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn restore(
    State(ctrl): State<Service>,
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
