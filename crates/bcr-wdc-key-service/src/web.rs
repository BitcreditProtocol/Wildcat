// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
use cashu::{nut01 as cdk01, nut02 as cdk02};
// ----- local imports
use crate::error::Result;
use crate::service::{KeysRepository, Service};

// ----- end imports

/// --------------------------- Look up keysets info
#[utoipa::path(
    get,
    path = "/v1/keysets/{kid}",
    params(
        ("kid" = cdk02::Id, Path, description = "The keyset id")
    ),
    responses (
        (status = 200, description = "Successful response", body = cdk02::KeySetInfo, content_type = "application/json"),
        (status = 404, description = "keyset id not  found"),
    )
)]
pub async fn lookup_keysets<QuotesKeysRepo, KeysRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo>>,
    Path(kid): Path<cdk02::Id>,
) -> Result<Json<cdk02::KeySetInfo>>
where
    KeysRepo: KeysRepository,
{
    log::debug!("Received keyset lookup request for id: {}", kid);

    let info = ctrl.info(kid).await?;
    Ok(Json(info.into()))
}

/// --------------------------- list keysets info
#[utoipa::path(
    get,
    path = "/v1/keysets",
    params(),
    responses (
        (status = 200, description = "Successful response", body = cdk02::KeysetResponse, content_type = "application/json"),
    )
)]
pub async fn list_keysets<QuotesKeysRepo, KeysRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo>>,
) -> Result<Json<cdk02::KeysetResponse>>
where
    KeysRepo: KeysRepository,
{
    log::debug!("Received keyset list request");

    let infos = ctrl
        .list_info()
        .await?
        .into_iter()
        .map(cdk02::KeySetInfo::from)
        .collect();
    let response = cdk02::KeysetResponse { keysets: infos };
    Ok(Json(response))
}

/// --------------------------- Look up keys
#[utoipa::path(
    get,
    path = "/v1/keys/{kid}",
    params(
        ("kid" = cdk02::Id, Path, description = "The keyset id")
    ),
    responses (
        (status = 200, description = "Successful response", body = cdk02::KeySet, content_type = "application/json"),
        (status = 404, description = "keyset id not  found"),
    )
)]
pub async fn lookup_keys<QuotesKeysRepo, KeysRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo>>,
    Path(kid): Path<cdk02::Id>,
) -> Result<Json<cdk02::KeySet>>
where
    KeysRepo: KeysRepository,
{
    log::debug!("Received keyset lookup request for id: {}", kid);

    let keyset = ctrl.keys(kid).await?;
    Ok(Json(keyset.into()))
}

/// --------------------------- List keys
#[utoipa::path(
    get,
    path = "/v1/keys",
    params(),
    responses (
        (status = 200, description = "Successful response", body = cdk01::KeysResponse, content_type = "application/json"),
    )
)]
pub async fn list_keys<QuotesKeysRepo, KeysRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo>>,
) -> Result<Json<cdk01::KeysResponse>>
where
    KeysRepo: KeysRepository,
{
    log::debug!("Received keyset list request ");

    let keysets = ctrl
        .list_keyset()
        .await?
        .into_iter()
        .map(cashu::KeySet::from)
        .collect();
    let response = cdk01::KeysResponse { keysets };
    Ok(Json(response))
}
