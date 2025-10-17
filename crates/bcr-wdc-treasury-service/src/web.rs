// ----- standard library imports
use axum::extract::{Json, State};
use bcr_wdc_webapi::exchange as web_exchange;
use cashu::nut03 as cdk03;
// ----- extra library imports
// ----- local imports
use crate::{foreign, debit, error::Result};

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
#[axum::debug_handler]
pub async fn online_exchange(
    State(ctrl): State<foreign::crsat::Service>,
    Json(request): Json<web_exchange::OnlineExchangeRequest>,
) -> Result<Json<web_exchange::OnlineExchangeResponse>> {
    tracing::debug!("Received request to online exchange");

    let signatures = ctrl
        .online_exchange(request.proofs, &request.exchange_path)
        .await?;
    let response = web_exchange::OnlineExchangeResponse { proofs: signatures };
    Ok(Json(response))
}
