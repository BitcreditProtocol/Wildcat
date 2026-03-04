// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_common::{
    cashu,
    core::BillId,
    wire::{
        exchange as wire_exchange, signatures as wire_signatures, treasury as wire_treasury,
        wallet as wire_wallet,
    },
};
// ----- local imports
use crate::{credit, debit, error::Result, foreign};
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
) -> Result<Json<wire_wallet::ECashBalance>> {
    tracing::debug!("Received request to sat_balance");

    let amount = ctrl.balance().await?;
    let response = wire_wallet::ECashBalance {
        amount,
        unit: cashu::CurrencyUnit::Sat,
    };
    Ok(Json(response))
}

pub async fn crsat_try_htlc_swap(
    State(ctrl): State<Arc<foreign::crsat::Service>>,
    Json(request): Json<wire_exchange::HtlcSwapAttemptRequest>,
) -> Result<Json<cashu::Amount>> {
    tracing::debug!("Received request to try_htlc_swap");

    let amount = ctrl.try_swap_htlc(&request.preimage).await?;
    Ok(Json(amount))
}

pub async fn sat_try_htlc_swap(
    State(ctrl): State<Arc<foreign::sat::Service>>,
    Json(request): Json<wire_exchange::HtlcSwapAttemptRequest>,
) -> Result<Json<cashu::Amount>> {
    tracing::debug!("Received request to try_htlc_swap");

    let amount = ctrl.try_swap_htlc(&request.preimage).await?;
    Ok(Json(amount))
}

pub async fn is_ebill_minting_completed(
    State(ctrl): State<debit::Service>,
    Path(bill_id): Path<BillId>,
) -> Result<Json<wire_wallet::EbillPaymentComplete>> {
    tracing::debug!("Received request for ebill payment completed {bill_id}");

    let complete = ctrl.is_ebill_payment_minted(bill_id).await?;
    let response = wire_wallet::EbillPaymentComplete { complete };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn new_mintop(
    State(ctrl): State<Arc<credit::Service>>,
    Json(request): Json<wire_treasury::NewMintOperationRequest>,
) -> Result<Json<wire_treasury::NewMintOperationResponse>> {
    tracing::debug!("Received new mint operation request");

    ctrl.new_minting_operation(
        request.quote_id,
        request.kid,
        request.pub_key,
        request.target,
        request.bill_id,
    )
    .await?;
    let response = wire_treasury::NewMintOperationResponse {};
    Ok(Json(response))
}

fn convert_mintop_status(status: credit::MintOperation) -> wire_treasury::MintOperationStatus {
    wire_treasury::MintOperationStatus {
        kid: status.kid,
        quote_id: status.uid,
        target: status.target,
        current: status.minted,
    }
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn mintop_status(
    State(ctrl): State<Arc<credit::Service>>,
    Path(qid): Path<uuid::Uuid>,
) -> Result<Json<wire_treasury::MintOperationStatus>> {
    tracing::debug!("Received mint operation status request {qid}");

    let status = ctrl.mintop_status(qid).await?;
    let status = convert_mintop_status(status);
    Ok(Json(status))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_mintops(
    State(ctrl): State<Arc<credit::Service>>,
    Path(kid): Path<cashu::Id>,
) -> Result<Json<Vec<uuid::Uuid>>> {
    tracing::debug!("Received list mint operations request");

    let mint_ops = ctrl.list_mintops_for_kid(kid).await?;
    let response = mint_ops.into_iter().map(|mop| mop.uid).collect();
    Ok(Json(response))
}
