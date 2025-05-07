// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_wdc_webapi::wallet::Balance;
// ----- local imports
use crate::error::Result;
use crate::service::{OnChainWallet, Service};

// ----- end imports

/// --------------------------- Look up keysets info
#[utoipa::path(
    get,
    path = "/v1/onchain/balance",
    params(
    ),
    responses (
        (status = 200, description = "Successful response", body = Balance, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn balance<OnChainWlt, PayRepo, EBillCl>(
    State(ctrl): State<Arc<Service<OnChainWlt, PayRepo, EBillCl>>>,
) -> Result<Json<Balance>>
where
    OnChainWlt: OnChainWallet,
{
    tracing::debug!("Received balance");

    let info = ctrl.balance().await?;
    Ok(Json(info.into()))
}
