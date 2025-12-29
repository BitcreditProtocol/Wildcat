// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_common::wire::{exchange as wire_exchange, melt as wire_melt, mint as wire_mint};
use bitcoin::base64::prelude::*;
use cashu::nut03 as cdk03;
use cdk::wallet::MintConnector;
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
    let response = ctrl.create_onchain_melt_quote(request).await?;
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn melt_onchain(
    State(ctrl): State<AppController>,
    Json(request): Json<cashu::MeltRequest<String>>,
) -> Result<Json<()>> {
    tracing::info!("Received melt_onchain request");
    let quote_id_str = request.quote_id();
    let quote_id = Uuid::parse_str(quote_id_str)
        .map_err(|_| crate::error::Error::InvalidInput(String::from("Invalid quote ID")))?;
    tracing::info!("Loading onchain melt quote with ID {}", quote_id);
    let onchain_request = ctrl.debit.repo.load_onchain_melt(quote_id).await?;
    let inputs = request.inputs();
    if inputs.is_empty() {
        return Err(crate::error::Error::InvalidInput(String::from("No inputs")));
    }

    let total_proofs: u64 = request
        .inputs_amount()
        .map_err(|_| crate::error::Error::InvalidInput(String::from("No amount for inputs")))?
        .into();
    if total_proofs != onchain_request.request.amount.to_sat() {
        return Err(crate::error::Error::InvalidInput(String::from(
            "Requested amount mismatch",
        )));
    }
    if let Some(clowder) = ctrl.clwdr_nats {
        tracing::info!("Requesting onchain clowder melt transaction");
        clowder.melt_onchain(request, onchain_request).await?;
    }
    Ok(Json(()))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn mint_quote_onchain(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_mint::MintQuoteOnchainRequest>,
) -> Result<Json<wire_mint::MintQuoteOnchainResponse>> {
    tracing::info!("Received mint_quote_onchain request");

    let clowder_quote = Uuid::new_v4();
    let address_response = ctrl
        .clwdr_rest
        .request_mint_address(&clowder_quote.to_string())
        .await?;

    let expiry = (chrono::Utc::now().timestamp() + ctrl.debit.quote_expiry_seconds as i64) as u64;
    let description = format!("clowder:{}", clowder_quote);
    let quote_request = cashu::MintQuoteBolt11Request {
        amount: cashu::Amount::from(request.amount.to_sat()),
        unit: cashu::CurrencyUnit::Sat,
        description: Some(description),
        pubkey: None,
    };
    let cdk_response = ctrl.dbmint.post_mint_quote(quote_request).await?;
    let cdk_quote = Uuid::parse_str(&cdk_response.quote)
        .map_err(|_| crate::error::Error::InvalidInput(String::from("Invalid CDK quote ID")))?;

    let data = debit::ClowderMintQuoteOnchain {
        clowder_quote,
        cdk_quote,
        address: address_response.address.clone(),
        amount: cashu::Amount::from(request.amount.to_sat()),
        expiry,
    };
    ctrl.debit.repo.store_onchain_mint(cdk_quote, data).await?;

    let wallet_response = wire_mint::MintQuoteOnchainResponse {
        quote: cdk_quote,
        address: address_response.address,
        amount: request.amount,
        expiry,
    };
    Ok(Json(wallet_response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_mint_quote_onchain(
    State(ctrl): State<AppController>,
    Path(quote_id): Path<String>,
) -> Result<Json<wire_mint::MintQuoteOnchainResponse>> {
    tracing::debug!("Received get_mint_quote_onchain request");
    let response = ctrl.debit.get_onchain_mint_quote(&quote_id).await?;
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn mint_onchain(
    State(ctrl): State<AppController>,
    Json(request): Json<cashu::MintRequest<String>>,
) -> Result<Json<cashu::MintResponse>> {
    tracing::debug!("Received mint_onchain request");

    let cdk_quote = Uuid::parse_str(&request.quote)
        .map_err(|_| crate::error::Error::InvalidInput(String::from("Invalid quote ID")))?;
    let data = ctrl.debit.repo.load_onchain_mint(cdk_quote).await?;

    let payment_response = ctrl
        .clwdr_rest
        .verify_mint_payment(&data.clowder_quote.to_string(), 1)
        .await?;

    tracing::info!("Clowder payment check {:?} sats", payment_response.amount);

    let outputs_amount = request
        .outputs
        .iter()
        .fold(cashu::Amount::ZERO, |acc, o| acc + o.amount);

    if outputs_amount > payment_response.amount {
        return Err(crate::error::Error::InvalidInput(format!(
            "Amount mismatch: outputs {}, insufficient payment {}",
            outputs_amount, payment_response.amount
        )));
    }

    let mint_request = cashu::MintRequest {
        quote: data.cdk_quote.to_string(),
        outputs: request.outputs,
        signature: None,
    };
    let response = ctrl.dbmint.post_mint(mint_request).await?;
    tracing::info!("Response signatures {:?}", response);

    Ok(Json(response))
}
