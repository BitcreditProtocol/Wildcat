// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_common::{
    cashu,
    wire::{exchange as wire_exchange, treasury as wire_treasury},
};
// ----- local imports
use crate::{ebill, error::Result, foreign, vault};
// ----- end imports

// ----- sat APIs
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn request_to_pay_ebill(
    State(ctrl): State<Arc<ebill::Service>>,
    Json(request): Json<wire_treasury::RequestToPayFromEBillRequest>,
) -> Result<Json<wire_treasury::RequestToPayFromEBillResponse>> {
    tracing::debug!("Received request to pay from ebill");

    ctrl.request_to_pay_ebill(request.ebill_id, request.amount, request.deadline)
        .await?;

    let response = wire_treasury::RequestToPayFromEBillResponse {};
    Ok(Json(response))
}

pub async fn try_htlc_swap(
    State(ctrl): State<Arc<foreign::Service>>,
    Json(request): Json<wire_exchange::HtlcSwapAttemptRequest>,
) -> Result<Json<cashu::Amount>> {
    let now = chrono::Utc::now();
    let amount = ctrl.try_swap_htlc(&request.preimage, now).await?;
    Ok(Json(amount))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn new_ebill_mintop(
    State(ctrl): State<Arc<ebill::Service>>,
    Json(request): Json<wire_treasury::NewMintOperationRequest>,
) -> Result<Json<wire_treasury::NewMintOperationResponse>> {
    let now = chrono::Utc::now();
    ctrl.new_minting_operation(
        request.quote_id,
        request.kid,
        request.pub_key,
        request.target,
        request.bill_id,
        now,
    )
    .await?;
    let response = wire_treasury::NewMintOperationResponse {};
    Ok(Json(response))
}

fn convert_ebill_mintop_status(status: ebill::MintOperation) -> wire_treasury::MintOperationStatus {
    wire_treasury::MintOperationStatus {
        kid: status.kid,
        quote_id: status.uid,
        target: status.target,
        current: status.minted,
    }
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn ebill_mintop_status(
    State(ctrl): State<Arc<ebill::Service>>,
    Path(qid): Path<uuid::Uuid>,
) -> Result<Json<wire_treasury::MintOperationStatus>> {
    let status = ctrl.mintop_status(qid).await?;
    let status = convert_ebill_mintop_status(status);
    Ok(Json(status))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_ebill_mintops(
    State(ctrl): State<Arc<ebill::Service>>,
    Path(kid): Path<cashu::Id>,
) -> Result<Json<Vec<uuid::Uuid>>> {
    let mint_ops = ctrl.list_mintops_for_kid(kid).await?;
    let response = mint_ops.into_iter().map(|mop| mop.uid).collect();
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn store_fees_proofs(
    State(ctrl): State<Arc<vault::Service>>,
    Json(request): Json<wire_treasury::StoreProofsRequest>,
) -> Result<Json<wire_treasury::StoreProofsResponse>> {
    ctrl.store_proofs(request.proofs).await?;
    let response = wire_treasury::StoreProofsResponse {};
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn generate_fees_token(
    State(ctrl): State<Arc<vault::Service>>,
) -> Result<Json<wire_treasury::FeesTokenResponse>> {
    let now = chrono::Utc::now();
    let token = ctrl.generate_token(now).await?;
    let total = token.value()?;
    let response = wire_treasury::FeesTokenResponse {
        token: token.to_string(),
        total,
    };
    Ok(Json(response))
}
