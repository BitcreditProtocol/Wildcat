// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::{
    cashu,
    wire::{
        clowder::messages as clowder_messages, exchange as wire_exchange, melt as wire_melt,
        mint as wire_mint,
    },
};
use bitcoin::base64::prelude::*;
use uuid::Uuid;
// ----- local imports
use crate::{ebill, error::Result, foreign, onchain, AppController};

// ----- end imports

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn online_exchange(
    State(ctrl): State<Arc<foreign::sat::Service>>,
    Json(request): Json<wire_exchange::OnlineExchangeRequest>,
) -> Result<Json<wire_exchange::OnlineExchangeResponse>> {
    tracing::debug!("Received request to online exchange");

    let exchange_path: Vec<cashu::PublicKey> = request
        .exchange_path
        .iter()
        .map(|p| cashu::PublicKey::from(*p))
        .collect();
    let signatures = ctrl.online_exchange(request.proofs, &exchange_path).await?;
    let response = wire_exchange::OnlineExchangeResponse { proofs: signatures };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn offline_exchange(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_exchange::OfflineExchangeRequest>,
) -> Result<Json<wire_exchange::OfflineExchangeResponse>> {
    tracing::debug!("Received request to offline exchange");

    let proofs = ctrl
        .sat
        .offline_exchange(request.fingerprints, request.hashes, request.wallet_pk)
        .await?;
    let payload = wire_exchange::OfflineExchangePayload { proofs };
    let serialized = borsh::to_vec(&payload)?;
    let request = clowder_messages::OfflineExchangeSignRequest {
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

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn melt_quote_onchain(
    State(ctrl): State<Arc<onchain::Service>>,
    Json(request): Json<wire_melt::MeltQuoteOnchainRequest>,
) -> Result<Json<wire_melt::MeltQuoteOnchainResponse>> {
    tracing::debug!("Received melt_quote_onchain request");

    let now = chrono::Utc::now();
    let response = ctrl.create_onchain_melt_quote(request, now).await?;
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn melt_onchain(
    State(ctrl): State<Arc<onchain::Service>>,
    Json(request): Json<wire_melt::MeltOnchainRequest>,
) -> Result<Json<wire_melt::MeltOnchainResponse>> {
    tracing::debug!("Received melt_onchain request");

    let now = chrono::Utc::now();
    let response = ctrl.melt_onchain(request, now).await?;
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
pub async fn mint_ebill(
    State(ctrl): State<Arc<ebill::Service>>,
    Json(request): Json<cashu::MintRequest<Uuid>>,
) -> Result<Json<cashu::MintResponse>> {
    let response = ctrl.mint(request).await?;
    Ok(Json(response))
}
