// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::wire::{exchange as wire_exchange, melt as wire_melt};
use bitcoin::base64::prelude::*;
use cashu::nut03 as cdk03;
use uuid::Uuid;
// ----- local imports
use crate::debit::Repository;
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
pub async fn sat_online_exchange(
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
pub async fn crsat_offline_exchange(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_exchange::OfflineExchangeRequest>,
) -> Result<Json<wire_exchange::OfflineExchangeResponse>> {
    tracing::debug!("Received request to offline exchange");

    let proofs = ctrl
        .crsat
        .offline_exchange(request.fingerprints, request.hashes, request.wallet_pk)
        .await?;
    let payload = wire_exchange::OfflineExchangePayload { proofs };
    let serialized = borsh::to_vec(&payload)?;
    let signature = ctrl.signer.sign_schnorr_preimage(&serialized).await?;
    let content = BASE64_STANDARD.encode(&serialized);
    let response = wire_exchange::OfflineExchangeResponse { content, signature };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn sat_offline_exchange(
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
    let signature = ctrl.signer.sign_schnorr_preimage(&serialized).await?;
    let content = BASE64_STANDARD.encode(&serialized);
    let response = wire_exchange::OfflineExchangeResponse { content, signature };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn melt_quote_onchain<Wlt, WdcSrvc, Repo>(
    State(ctrl): State<debit::Service<Wlt, WdcSrvc, Repo>>,
    Json(request): Json<wire_melt::MeltQuoteOnchainRequest>,
) -> Result<Json<cashu::nuts::MeltQuoteBolt11Response<String>>>
where
    Repo: debit::Repository,
{
    let expiry = chrono::Utc::now().timestamp() + 86400;
    let quote_id = Uuid::new_v4();
    ctrl.repo
        .store_onchain_melt(quote_id, request.clone())
        .await?;
    Ok(Json(cashu::nuts::MeltQuoteBolt11Response {
        quote: quote_id.to_string(),
        fee_reserve: cashu::Amount::ZERO,
        paid: Some(false),
        payment_preimage: None,
        change: None,
        amount: request.request.amount,
        unit: Some(request.unit),
        request: None,
        state: cashu::nuts::MeltQuoteState::Unpaid,
        expiry: expiry as u64,
    }))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn melt_onchain(
    State(ctrl): State<AppController>,
    Json(request): Json<cashu::MeltRequest<String>>,
) -> Result<Json<()>> {
    let quote_id_str = request.quote_id();
    let quote_id = Uuid::parse_str(quote_id_str)
        .map_err(|_| crate::error::Error::InvalidInput(String::from("Invalid quote ID")))?;
    let onchain_request = ctrl.debit.repo.load_onchain_melt(quote_id).await?;
    let inputs = request.inputs();
    if inputs.is_empty() {
        return Err(crate::error::Error::InvalidInput(String::from("No inputs")));
    }
    let total_proofs = request
        .inputs_amount()
        .map_err(|_| crate::error::Error::InvalidInput(String::from("No amount for inputs")))?;
    if total_proofs != onchain_request.request.amount {
        return Err(crate::error::Error::InvalidInput(String::from(
            "Requested amount mismatch",
        )));
    }
    if let Some(clowder) = ctrl.clwdr_nats {
        clowder.melt_onchain(request, onchain_request).await?;
    }
    Ok(Json(()))
}
