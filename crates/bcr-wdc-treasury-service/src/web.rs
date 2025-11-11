// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_wdc_webapi::exchange as web_exchange;
use bitcoin::base64::prelude::*;
use cashu::nut03 as cdk03;
// ----- local imports
use crate::{debit, error::Result, foreign, AppController};

// ----- end imports

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn redeem<Wlt, WdcSrvc, Repo>(
    State(ctrl): State<debit::Service<Wlt, WdcSrvc, Repo>>,
    Json(request): Json<cdk03::SwapRequest>,
) -> Result<Json<cdk03::SwapResponse>>
where
    Wlt: debit::Wallet,
    WdcSrvc: debit::WildcatService,
{
    tracing::debug!("Received request to redeem");

    let signatures = ctrl.redeem(request.inputs(), request.outputs()).await?;
    let response = cdk03::SwapResponse { signatures };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn crsat_online_exchange(
    State(ctrl): State<Arc<foreign::crsat::Service>>,
    Json(request): Json<web_exchange::OnlineExchangeRequest>,
) -> Result<Json<web_exchange::OnlineExchangeResponse>> {
    tracing::debug!("Received request to online exchange");

    let exchange_path: Vec<cashu::PublicKey> = request
        .exchange_path
        .iter()
        .map(|p| cashu::PublicKey::from(*p))
        .collect();
    let signatures = ctrl.online_exchange(request.proofs, &exchange_path).await?;
    let response = web_exchange::OnlineExchangeResponse { proofs: signatures };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn sat_online_exchange(
    State(ctrl): State<Arc<foreign::sat::Service>>,
    Json(request): Json<web_exchange::OnlineExchangeRequest>,
) -> Result<Json<web_exchange::OnlineExchangeResponse>> {
    tracing::debug!("Received request to online exchange");

    let exchange_path: Vec<cashu::PublicKey> = request
        .exchange_path
        .iter()
        .map(|p| cashu::PublicKey::from(*p))
        .collect();
    let signatures = ctrl.online_exchange(request.proofs, &exchange_path).await?;
    let response = web_exchange::OnlineExchangeResponse { proofs: signatures };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn crsat_offline_exchange(
    State(ctrl): State<AppController>,
    Json(request): Json<web_exchange::OfflineExchangeRequest>,
) -> Result<Json<web_exchange::OfflineExchangeResponse>> {
    tracing::debug!("Received request to offline exchange");

    let proofs = ctrl
        .crsat
        .offline_exchange(request.fingerprints, request.hashes, request.wallet_pk)
        .await?;
    let payload = web_exchange::OfflineExchangePayload { proofs };
    let serialized = borsh::to_vec(&payload)?;
    let signature = ctrl.signer.sign_bytes(&serialized).await?;
    let content = BASE64_STANDARD.encode(&serialized);
    let response = web_exchange::OfflineExchangeResponse { content, signature };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn sat_offline_exchange(
    State(ctrl): State<AppController>,
    Json(request): Json<web_exchange::OfflineExchangeRequest>,
) -> Result<Json<web_exchange::OfflineExchangeResponse>> {
    tracing::debug!("Received request to offline exchange");

    let proofs = ctrl
        .sat
        .offline_exchange(request.fingerprints, request.hashes, request.wallet_pk)
        .await?;
    let payload = web_exchange::OfflineExchangePayload { proofs };
    let serialized = borsh::to_vec(&payload)?;
    let signature = ctrl.signer.sign_bytes(&serialized).await?;
    let content = BASE64_STANDARD.encode(&serialized);
    let response = web_exchange::OfflineExchangeResponse { content, signature };
    Ok(Json(response))
}
