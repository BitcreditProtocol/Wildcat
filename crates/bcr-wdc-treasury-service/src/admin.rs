// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::wire::signatures as wire_signatures;
use bcr_wdc_webapi::{exchange as web_exchange, wallet as web_wallet};
use cashu::{self as cdk};
// ----- local imports
use crate::{debit, error::Result, foreign};
// ----- end imports

// ----- sat APIs
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn request_to_pay_ebill<Wlt, WdcSrvc, Repo>(
    State(ctrl): State<debit::Service<Wlt, WdcSrvc, Repo>>,
    Json(request): Json<wire_signatures::RequestToMintFromEBillRequest>,
) -> Result<Json<wire_signatures::RequestToMintFromEBillResponse>>
where
    Wlt: debit::Wallet + 'static,
    WdcSrvc: debit::WildcatService + 'static,
    Repo: debit::Repository + 'static,
{
    tracing::debug!("Received request to mint from ebill");

    let quote = ctrl
        .mint_from_ebill(
            request.ebill_id,
            cashu::Amount::from(request.amount.to_sat()),
            request.deadline,
        )
        .await?;
    let response = wire_signatures::RequestToMintFromEBillResponse {
        request_id: quote.id,
        request: quote.request,
    };
    Ok(Json(response))
}

pub async fn sat_balance<Wlt, WdcSrvc, Repo>(
    State(ctrl): State<debit::Service<Wlt, WdcSrvc, Repo>>,
) -> Result<Json<web_wallet::ECashBalance>>
where
    Wlt: debit::Wallet,
{
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
