// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use axum::extract::{Json, Path, State};
use cashu::{nut01 as cdk01, nut02 as cdk02, nut04 as cdk04};
// ----- local imports
use crate::error::{Error, Result};
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
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn lookup_keysets<QuotesKeysRepo, KeysRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo>>,
    Path(kid): Path<cdk02::Id>,
) -> Result<Json<cdk02::KeySetInfo>>
where
    KeysRepo: KeysRepository,
{
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
        (status = 200, description = "Successful response", body = cdk02::KeysetResponse, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_keysets<QuotesKeysRepo, KeysRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo>>,
) -> Result<Json<cdk02::KeysetResponse>>
where
    KeysRepo: KeysRepository,
{
    tracing::debug!("Received keysets list request");

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
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn lookup_keys<QuotesKeysRepo, KeysRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo>>,
    Path(kid): Path<cdk02::Id>,
) -> Result<Json<cdk02::KeySet>>
where
    KeysRepo: KeysRepository,
{
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
        (status = 200, description = "Successful response", body = cdk01::KeysResponse, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_keys<QuotesKeysRepo, KeysRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo>>,
) -> Result<Json<cdk01::KeysResponse>>
where
    KeysRepo: KeysRepository,
{
    tracing::debug!("Received keys list request");

    let keysets = ctrl
        .list_keyset()
        .await?
        .into_iter()
        .map(cashu::KeySet::from)
        .collect();
    let response = cdk01::KeysResponse { keysets };
    Ok(Json(response))
}

/// --------------------------- Mint
#[utoipa::path(
    post,
    path = "/v1/mint/ebill",
    request_body(content = cdk04::MintBolt11Request<String>, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = cdk04::MintBolt11Response, content_type = "application/json"),
    )
)]
pub async fn mint_ebill<QuotesKeysRepo, KeysRepo>(
    State(ctrl): State<Service<QuotesKeysRepo, KeysRepo>>,
    Json(req): Json<cdk04::MintBolt11Request<String>>,
) -> Result<Json<cdk04::MintBolt11Response>>
where
    KeysRepo: KeysRepository,
{
    tracing::debug!("Received mint request for");

    let kid = req
        .outputs
        .first()
        .ok_or(Error::InvalidMintRequest)?
        .keyset_id;
    let pk = ctrl.authorized_public_key_to_mint(kid).await?;
    req.verify_signature(pk)
        .map_err(|_| Error::InvalidMintRequest)?;

    let qid = uuid::Uuid::from_str(&req.quote).map_err(|_| Error::InvalidMintRequest)?;
    let signatures = ctrl.mint(qid, req.outputs).await?;
    let response = cdk04::MintBolt11Response { signatures };
    Ok(Json(response))
}
