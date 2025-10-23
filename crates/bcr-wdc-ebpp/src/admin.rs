// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_wdc_webapi::wallet::Balance;
// ----- local imports
use crate::error::Result;
use crate::service::Service;

// ----- end imports

/// --------------------------- Look up keysets info
#[utoipa::path(
    get,
    path = "/v1/admin/ebpp/onchain/balance",
    params(
    ),
    responses (
        (status = 200, description = "Successful response", body = Balance, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn balance(State(ctrl): State<Arc<Service>>) -> Result<Json<Balance>> {
    tracing::debug!("Received balance");

    let info = ctrl.balance().await?;
    Ok(Json(info.into()))
}
