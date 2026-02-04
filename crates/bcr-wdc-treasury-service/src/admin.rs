// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_common::{core::BillId, wire::signatures as wire_signatures};
use bcr_wdc_webapi::{exchange as web_exchange, wallet as web_wallet};
use cashu::{self as cdk};
// ----- local imports
use crate::{debit, error::Result, foreign};
// ----- end imports

// ----- sat APIs
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn request_to_pay_ebill(
    State(ctrl): State<debit::Service>,
    Json(request): Json<wire_signatures::RequestToMintFromEBillRequest>,
) -> Result<Json<wire_signatures::RequestToMintFromEBillResponse>> {
    tracing::debug!("Received request to mint from ebill");

    let quote = ctrl
        .mint_from_ebill(request.ebill_id, request.amount, request.deadline)
        .await?;
    let response = wire_signatures::RequestToMintFromEBillResponse {
        request_id: quote.id,
        request: quote.request,
    };
    Ok(Json(response))
}

pub async fn sat_balance(
    State(ctrl): State<debit::Service>,
) -> Result<Json<web_wallet::ECashBalance>> {
    tracing::debug!("Received request to sat_balance");

    let amount = ctrl.balance().await?;
    let response = web_wallet::ECashBalance {
        amount,
        unit: cdk::CurrencyUnit::Sat,
    };
    Ok(Json(response))
}

pub async fn crsat_try_htlc_swap(
    State(ctrl): State<Arc<foreign::crsat::Service>>,
    Json(request): Json<web_exchange::HtlcSwapAttemptRequest>,
) -> Result<Json<cashu::Amount>> {
    tracing::debug!("Received request to try_htlc_swap");

    let amount = ctrl.try_swap_htlc(&request.preimage).await?;
    Ok(Json(amount))
}

pub async fn sat_try_htlc_swap(
    State(ctrl): State<Arc<foreign::sat::Service>>,
    Json(request): Json<web_exchange::HtlcSwapAttemptRequest>,
) -> Result<Json<cashu::Amount>> {
    tracing::debug!("Received request to try_htlc_swap");

    let amount = ctrl.try_swap_htlc(&request.preimage).await?;
    Ok(Json(amount))
}

pub async fn is_ebill_minting_completed(
    State(ctrl): State<debit::Service>,
    Path(bill_id): Path<BillId>,
) -> Result<Json<web_wallet::EbillPaymentComplete>> {
    tracing::debug!("Received request for ebill payment completed {bill_id}");

    let complete = ctrl.is_ebill_payment_minted(bill_id).await?;
    let response = web_wallet::EbillPaymentComplete { complete };
    Ok(Json(response))
}
