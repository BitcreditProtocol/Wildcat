// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::{
    cashu,
    wire::{
        clowder as wire_clowder, exchange as wire_exchange, melt as wire_melt, mint as wire_mint,
    },
};
use bcr_wdc_utils::nut19;
use bitcoin::base64::prelude::*;
use uuid::Uuid;
// ----- local imports
use crate::{ebill, error::Result, foreign, onchain, vault, AppController};

// ----- end imports

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn online_exchange(
    State(ctrl): State<Arc<foreign::Service>>,
    Json(request): Json<wire_exchange::OnlineExchangeRequest>,
) -> Result<Json<wire_exchange::OnlineExchangeResponse>> {
    let wire_exchange::OnlineExchangeRequest {
        proofs,
        exchange_path,
    } = request;

    let signatures = ctrl.online_exchange(proofs, exchange_path).await?;
    let response = wire_exchange::OnlineExchangeResponse { proofs: signatures };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn offline_exchange(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_exchange::OfflineExchangeRequest>,
) -> Result<Json<wire_exchange::OfflineExchangeResponse>> {
    let proofs = ctrl
        .foreign
        .offline_exchange(request.fingerprints, request.hashes, request.wallet_pk)
        .await?;
    let payload = wire_exchange::OfflineExchangePayload { proofs };
    let serialized = borsh::to_vec(&payload)?;
    let request = wire_clowder::OfflineExchangeSignRequest {
        payload: serialized.clone(),
    };
    let signature = ctrl
        .clwdr_nats
        .sign_offline_exchange(request)
        .await?
        .signature;
    let content = BASE64_STANDARD.encode(&serialized);
    let response = wire_exchange::OfflineExchangeResponse { content, signature };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, cache))]
pub async fn melt_quote_onchain(
    State(ctrl): State<Arc<onchain::Service>>,
    State(cache): State<Arc<dyn nut19::Cache>>,
    Json(request): Json<wire_melt::MeltQuoteOnchainRequest>,
) -> Result<Json<wire_melt::MeltQuoteOnchainResponse>> {
    let now = chrono::Utc::now();
    let key = nut19::onchain::melt_quote::request_to_key(request.clone());
    if let Some(blob) = cache.load(key).await {
        let response = nut19::onchain::melt_quote::blob_to_response(blob);
        return Ok(Json(response));
    }
    let response = ctrl.create_onchain_melt_quote(request, now).await?;
    let blob = nut19::onchain::melt_quote::response_to_blob(&response);
    cache.store_and_clean(key, blob, now).await;
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, vault_srvc, cache))]
pub async fn melt_onchain(
    State(ctrl): State<Arc<onchain::Service>>,
    State(vault_srvc): State<Arc<vault::Service>>,
    State(cache): State<Arc<dyn nut19::Cache>>,
    Json(request): Json<wire_melt::MeltOnchainRequest>,
) -> Result<Json<wire_melt::MeltOnchainResponse>> {
    let now = chrono::Utc::now();
    let key = nut19::onchain::melt::request_to_key(request.clone());
    if let Some(blob) = cache.load(key).await {
        let response = nut19::onchain::melt::blob_to_response(blob);
        return Ok(Json(response));
    }
    let vault = onchain::VaultSrvc { vault: vault_srvc };
    let response = ctrl.melt_onchain(request, now, &vault).await?;
    let blob = nut19::onchain::melt::response_to_blob(&response);
    cache.store_and_clean(key, blob, now).await;
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn mint_quote_onchain(
    State(ctrl): State<Arc<onchain::Service>>,
    Json(request): Json<wire_mint::OnchainMintQuoteRequest>,
) -> Result<Json<wire_mint::OnchainMintQuoteResponse>> {
    let now = chrono::Utc::now();
    let response = ctrl.create_onchain_mint_quote(request, now).await?;
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn mint_onchain(
    State(ctrl): State<Arc<onchain::Service>>,
    Json(request): Json<wire_mint::OnchainMintRequest>,
) -> Result<Json<cashu::MintResponse>> {
    let signatures = ctrl.mint_onchain(request.quote, request.alpha_id).await?;
    let response = cashu::MintResponse { signatures };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, cache))]
pub async fn mint_ebill(
    State(ctrl): State<Arc<ebill::Service>>,
    State(cache): State<Arc<dyn nut19::Cache>>,
    Json(request): Json<cashu::MintRequest<Uuid>>,
) -> Result<Json<cashu::MintResponse>> {
    let now = chrono::Utc::now();
    let key = nut19::ebill::mint::request_to_key(request.clone());
    if let Some(blob) = cache.load(key).await {
        let response = nut19::ebill::mint::blob_to_response(blob);
        return Ok(Json(response));
    }
    let response = ctrl.mint(request).await?;
    let blob = nut19::ebill::mint::response_to_blob(&response);
    cache.store_and_clean(key, blob, now).await;
    Ok(Json(response))
}
