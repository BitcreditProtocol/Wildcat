// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_wdc_webapi::wallet::Balance;
// ----- local imports
use crate::error::Result;
use crate::service::{Bip39Wallet, Service};

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
pub async fn balance<Bip39Wlt>(State(ctrl): State<Arc<Service<Bip39Wlt>>>) -> Result<Json<Balance>>
where
    Bip39Wlt: Bip39Wallet,
{
    log::debug!("Received balance");

    let info = ctrl.onchain_balance().await?;
    Ok(Json(info.into()))
}
