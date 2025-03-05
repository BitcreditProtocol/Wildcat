// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
//use bcr_wdc_webapi::quotes as web_quotes;
//use bitcoin::hashes::sha256::Hash as Sha256;
//use bitcoin::hashes::Hash;
use cashu::nuts::nut02 as cdk02;
// ----- local imports
use crate::error::Result;
use crate::service::{KeysRepository, Service};

/// --------------------------- Look up keysets info
#[utoipa::path(
    get,
    path = "/v1/credit/keysets/{kid}",
    params(
        ("kid" = cdk02::Id, Path, description = "The keyset id")
    ),
    responses (
        (status = 200, description = "Successful response", body = StatusReply, content_type = "application/json"),
        (status = 404, description = "keyset id not  found"),
    )
)]
pub async fn lookup_keyset<KR>(
    State(ctrl): State<Service<KR>>,
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
    path = "/v1/credit/keys/{kid}",
    params(
        ("kid" = cdk02::Id, Path, description = "The keyset id")
    ),
    responses (
        (status = 200, description = "Successful response", body = StatusReply, content_type = "application/json"),
        (status = 404, description = "keyset id not  found"),
    )
)]
pub async fn lookup_keys<KR>(
    State(ctrl): State<Service<KR>>,
    Path(kid): Path<cdk02::Id>,
) -> Result<Json<cdk02::KeySet>>
where
    KR: KeysRepository,
{
    log::debug!("Received keyset lookup request for id: {}", kid);

    let keyset = ctrl.keys(kid).await?;
    Ok(Json(keyset.into()))
}
