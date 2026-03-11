// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::{
    cashu,
    cdk::wallet::MintConnector,
    wire::{clowder::messages, exchange as wire_exchange, melt as wire_melt, mint as wire_mint},
};
use bitcoin::base64::prelude::*;
// ----- local imports
use crate::{credit, debit, error::Result, foreign, AppController};

// ----- end imports

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn redeem(
    State(ctrl): State<debit::Service>,
    Json(request): Json<cashu::SwapRequest>,
) -> Result<Json<cashu::SwapResponse>> {
    tracing::debug!("Received request to redeem");

    let signatures = ctrl.redeem(request.inputs(), request.outputs()).await?;
    let response = cashu::SwapResponse { signatures };
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
pub async fn melt_quote_onchain(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_melt::MeltQuoteOnchainRequest>,
) -> Result<Json<wire_melt::MeltQuoteOnchainResponse>> {
    if ctrl.clwdr_nats.is_none() {
        return Err(crate::error::Error::ClowderUnavailable);
    }
    if request.request.amount < ctrl.params.min_melt_threshold {
        return Err(crate::error::Error::InsufficientOnchainMeltAmount(
            request.request.amount,
        ));
    }

    let response = ctrl.debit.create_onchain_melt_quote(request).await?;
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn melt_onchain(
    State(ctrl): State<AppController>,
    Json(request): Json<cashu::MeltRequest<uuid::Uuid>>,
) -> Result<Json<wire_melt::MeltQuoteOnchainResponse>> {
    tracing::debug!("Received melt_onchain request");
    let quote_id = request.quote_id();
    tracing::debug!("Loading onchain melt quote with ID {}", quote_id);
    let onchain_data = ctrl.debit.repo.load_onchain_melt(*quote_id).await?;

    let total_proofs: u64 = request
        .inputs_amount()
        .map_err(|_| crate::error::Error::InvalidInput(String::from("No amount for inputs")))?
        .into();

    tracing::info!(
        "On chain melt request id {} total inputs {} sat addr {} original quote amount {}",
        request.quote(),
        total_proofs,
        onchain_data
            .request
            .request
            .address
            .clone()
            .assume_checked(),
        onchain_data.request.request.amount
    );

    let current_time = chrono::Utc::now().timestamp() as u64;
    if current_time > onchain_data.expiry {
        return Err(crate::error::Error::InvalidInput(String::from(
            "Melt quote has expired",
        )));
    }
    let inputs = request.inputs();
    if inputs.is_empty() {
        return Err(crate::error::Error::InvalidInput(String::from("No inputs")));
    }

    if total_proofs != onchain_data.request.request.amount.to_sat() {
        return Err(crate::error::Error::MeltAmountMismatch(
            cashu::Amount::from(total_proofs),
        ));
    }

    let Some(clowder) = ctrl.clwdr_nats else {
        return Err(crate::error::Error::ClowderUnavailable);
    };

    let melt_request = cashu::MeltRequest::new(quote_id.to_string(), inputs.clone(), None);
    let cdk_resp = ctrl.dbmint.post_melt(melt_request).await?;
    if cdk_resp.paid != Some(true) {
        tracing::error!("Invalid cdk resp state {:?}", cdk_resp);
        return Err(crate::error::Error::Internal(
            "CDK Mintd did not mark melt quote as paid".to_string(),
        ));
    }

    tracing::debug!("Requesting onchain clowder melt transaction");
    let melt_resp = clowder
        .melt_onchain(messages::MeltOnchainRequest {
            quote: *quote_id,
            address: onchain_data.request.request.address,
            amount: onchain_data.request.request.amount,
            proofs: inputs.clone(),
        })
        .await?;

    let resp = wire_melt::MeltQuoteOnchainResponse {
        txid: Some(melt_resp.txid),
        quote: *quote_id,
        fee_reserve: bitcoin::Amount::ZERO,
        change: None,
        amount: onchain_data.request.request.amount,
        unit: Some(onchain_data.request.unit),
        state: cashu::nuts::MeltQuoteState::Paid,
        expiry: onchain_data.expiry,
    };

    Ok(Json(resp))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn mint_quote_onchain(
    State(ctrl): State<debit::Service>,
    Json(request): Json<wire_mint::OnchainMintQuoteRequest>,
) -> Result<Json<wire_mint::OnchainMintQuoteResponse>> {
    tracing::debug!("Received mint_quote_onchain request");

    let now = chrono::Utc::now();
    let response = ctrl.new_onchain_mintop(request, now).await?;
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn mint_ebill(
    State(ctrl): State<Arc<credit::Service>>,
    Json(req): Json<cashu::MintRequest<uuid::Uuid>>,
) -> Result<Json<cashu::MintResponse>> {
    tracing::debug!("Received mint request for {}", req.quote);

    let response = ctrl.mint(req).await?;
    Ok(Json(response))
}
