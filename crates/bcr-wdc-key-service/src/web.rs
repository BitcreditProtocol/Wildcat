// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
use cashu::nuts::nut02 as cdk02;
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
pub async fn lookup_keyset<QuotesKeysRepo, KeysRepo>(
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

/// --------------------------- Look up keys
#[utoipa::path(
    get,
    path = "/v1/keys/{kid}",
    params(
        ("kid" = cdk02::Id, Path, description = "The keyset id")
    ),
    responses (
        (status = 200, description = "Successful response", body = cdk02::Id, content_type = "application/json"),
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
