// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_common::{
    cashu,
    cdk::wallet::MintConnector,
    wire::{clowder::messages, exchange as wire_exchange, melt as wire_melt, mint as wire_mint},
};
use bitcoin::base64::prelude::*;
use uuid::Uuid;
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
    State(ctrl): State<AppController>,
    Json(request): Json<wire_mint::OnchainMintQuoteRequest>,
) -> Result<Json<wire_mint::OnchainMintQuoteResponse>> {
    tracing::debug!("Received mint_quote_onchain request");

    let amount_sat: u64 = request
        .blinded_messages
        .iter()
        .fold(cashu::Amount::ZERO, |acc, b| acc + b.amount)
        .into();
    let payment_amount = bitcoin::Amount::from_sat(amount_sat + 1); // 1 sat fee

    if payment_amount < ctrl.params.min_mint_threshold {
        return Err(crate::error::Error::InsufficientOnchainMintAmount(
            payment_amount,
        ));
    }

    let clowder_quote = Uuid::new_v4();
    let dummy_kid = cashu::Id::from_bytes(&[0_u8; 8])
        .map_err(|_| crate::error::Error::InvalidInput(String::from("Invalid keyset ID")))?;
    let address_response = ctrl
        .clwdr_rest
        .request_mint_address(clowder_quote, dummy_kid)
        .await?;

    let expiry = (chrono::Utc::now().timestamp() + ctrl.debit.quote_expiry_seconds as i64) as u64;
    let description = format!("clowder:{}", clowder_quote);
    let quote_request = cashu::MintQuoteBolt11Request {
        amount: cashu::Amount::from(amount_sat),
        unit: cashu::CurrencyUnit::Sat,
        description: Some(description),
        pubkey: None,
    };
    let cdk_response = ctrl.dbmint.post_mint_quote(quote_request).await?;
    let cdk_quote = Uuid::parse_str(&cdk_response.quote)
        .map_err(|_| crate::error::Error::InvalidInput(String::from("Invalid CDK quote ID")))?;

    let body = wire_mint::OnchainMintQuoteResponseBody {
        quote: cdk_quote,
        address: address_response.address.clone(),
        payment_amount,
        expiry,
        blinded_messages: request.blinded_messages.clone(),
    };
    let borsh_bytes = borsh::to_vec(&body)?;
    let commitment = ctrl.signer.sign_schnorr_preimage(&borsh_bytes).await?;

    let data = debit::ClowderMintQuoteOnchain {
        clowder_quote,
        cdk_quote,
        address: address_response.address,
        amount: cashu::Amount::from(amount_sat),
        expiry,
        blinded_messages: request.blinded_messages,
        commitment,
    };
    ctrl.debit.repo.store_onchain_mint(cdk_quote, data).await?;

    let content = BASE64_STANDARD.encode(&borsh_bytes);
    let wallet_response = wire_mint::OnchainMintQuoteResponse {
        content,
        commitment,
    };
    Ok(Json(wallet_response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_mint_quote_onchain(
    State(ctrl): State<AppController>,
    Path(quote_id): Path<String>,
) -> Result<Json<wire_mint::OnchainMintQuoteResponse>> {
    tracing::debug!("Received get_mint_quote_onchain request");

    let cdk_quote = Uuid::parse_str(&quote_id)
        .map_err(|_| crate::error::Error::InvalidInput(String::from("Invalid quote ID")))?;
    let data = ctrl.debit.repo.load_onchain_mint(cdk_quote).await?;

    let amount_sat: u64 = data.amount.into();
    let body = wire_mint::OnchainMintQuoteResponseBody {
        quote: data.cdk_quote,
        address: data.address,
        payment_amount: bitcoin::Amount::from_sat(amount_sat + 1), // 1 sat fee
        expiry: data.expiry,
        blinded_messages: data.blinded_messages,
    };
    let borsh_bytes = borsh::to_vec(&body)?;
    let content = BASE64_STANDARD.encode(&borsh_bytes);

    Ok(Json(wire_mint::OnchainMintQuoteResponse {
        content,
        commitment: data.commitment,
    }))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn mint_onchain(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_mint::OnchainMintRequest>,
) -> Result<Json<wire_mint::MintResponse>> {
    tracing::debug!("Received mint_onchain request");

    let cdk_quote = request.quote;
    let data = ctrl.debit.repo.load_onchain_mint(cdk_quote).await?;

    let current_time = chrono::Utc::now().timestamp() as u64;
    if current_time > data.expiry {
        return Err(crate::error::Error::InvalidInput(String::from(
            "Mint quote has expired",
        )));
    }

    let kid = data
        .blinded_messages
        .first()
        .ok_or(crate::error::Error::InvalidInput(String::from(
            "Missing output",
        )))?
        .keyset_id;

    let payment_response = ctrl
        .clwdr_rest
        .verify_mint_payment(data.clowder_quote, kid, ctrl.params.min_confirmations)
        .await?;

    tracing::debug!("Clowder payment check {:?} sats", payment_response.amount);

    let outputs_amount: u64 = data
        .blinded_messages
        .iter()
        .fold(cashu::Amount::ZERO, |acc, o| acc + o.amount)
        .into();

    tracing::info!("On chain mint cdk id {} clowder id {} total outputs {outputs_amount} payment received {}, original quote amount {}", data.cdk_quote, data.clowder_quote, payment_response.amount, outputs_amount);

    if outputs_amount != u64::from(data.amount) {
        return Err(crate::error::Error::MintAmountMismatch(
            cashu::Amount::from(outputs_amount),
        ));
    }

    if outputs_amount > payment_response.amount.to_sat() {
        return Err(crate::error::Error::InvalidInput(format!(
            "Amount mismatch: outputs {}, insufficient payment {}",
            outputs_amount, payment_response.amount
        )));
    }

    let mint_request = cashu::MintRequest {
        quote: data.cdk_quote.to_string(),
        outputs: data.blinded_messages,
        signature: None,
    };
    let response = ctrl.dbmint.post_mint(mint_request).await?;

    if let Some(clowder) = ctrl.clwdr_nats {
        let req = messages::MintOnchainRequest {
            keyset_id: kid,
            quote_id: data.clowder_quote,
            amount: cashu::Amount::from(outputs_amount),
        };
        let resp = messages::MintOnchainResponse {
            signatures: response.signatures.clone(),
        };
        clowder.mint_onchain(req, resp).await?;
        tracing::debug!("Sent mint to clowder");
    }

    Ok(Json(wire_mint::MintResponse {
        signatures: response.signatures,
    }))
}

pub async fn mint_ebill(
    State(ctrl): State<Arc<credit::Service>>,
    Json(req): Json<cashu::MintRequest<uuid::Uuid>>,
) -> Result<Json<cashu::MintResponse>> {
    tracing::debug!("Received mint request for {}", req.quote);

    let response = ctrl.mint(req).await?;
    Ok(Json(response))
}
