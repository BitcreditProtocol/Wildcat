// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_wdc_webapi::wallet::Network;
// ----- local imports
use crate::error::Result;
use crate::service::{OnChainWallet, Service};

// ----- end imports

/// --------------------------- Look up keysets info
#[utoipa::path(
    get,
    path = "/v1/ebpp/onchain/network",
    params(
    ),
    responses (
        (status = 200, description = "Successful response", body = Network, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn network<OnChainWlt, PayRepo, EBillCl>(
    State(ctrl): State<Arc<Service<OnChainWlt, PayRepo, EBillCl>>>,
) -> Result<Json<Network>>
where
    OnChainWlt: OnChainWallet,
{
    tracing::debug!("Received network request");

    let net = ctrl.network();
    Ok(Json(Network { network: net }))
}
