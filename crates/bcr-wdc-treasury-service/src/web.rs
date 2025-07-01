// ----- standard library imports
use axum::extract::{Json, State};
use cashu::nut03 as cdk03;
// ----- extra library imports
// ----- local imports
use crate::{debit, error::Result};

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
