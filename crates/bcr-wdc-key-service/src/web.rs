// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
use cashu::nuts::nut02 as cdk02;
// ----- local imports
use crate::error::Result;
use crate::service::{KeysRepository, Service};

/// --------------------------- Look up keysets info
#[utoipa::path(
    get,
    path = "/v1/keysets/{kid}",
    params(
        ("kid" = cdk02::Id, Path, description = "The keyset id")
    ),
    responses (
        (status = 200, description = "Successful response", body = StatusReply, content_type = "application/json"),
        (status = 404, description = "keyset id not  found"),
    )
)]
pub async fn lookup_keyset<QKR, KR>(
    State(ctrl): State<Service<QKR, KR>>,
    Path(kid): Path<cdk02::Id>,
) -> Result<Json<cdk02::KeySetInfo>>
where
    KR: KeysRepository,
{
    log::debug!("Received keyset lookup request for id: {}", kid);

    let info = ctrl.info(kid).await?;
    Ok(Json(info.into()))
}

/// --------------------------- Look up keys
#[utoipa::path(
    get,
    path = "/v1/keys/{kid}",
    params(
        ("kid" = cdk02::Id, Path, description = "The keyset id")
    ),
    responses (
        (status = 200, description = "Successful response", body = StatusReply, content_type = "application/json"),
        (status = 404, description = "keyset id not  found"),
    )
)]
pub async fn lookup_keys<QKR, KR>(
    State(ctrl): State<Service<QKR, KR>>,
    Path(kid): Path<cdk02::Id>,
) -> Result<Json<cdk02::KeySet>>
where
    KR: KeysRepository,
{
    log::debug!("Received keyset lookup request for id: {}", kid);

    let keyset = ctrl.keys(kid).await?;
    Ok(Json(keyset.into()))
}
