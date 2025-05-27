// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_wdc_webapi::eiou as web_eiou;
// ----- local imports
use crate::{error::Result, AppController};

// ----- end imports

/// --------------------------- get e-IOU balance
#[utoipa::path(
    get,
    path = "/v1/eiou/balance",
    params(
    ),
    responses (
        (status = 200, description = "Successful response", body = web_eiou::BalanceResponse, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(_ctrl))]
pub async fn balance(
    State(_ctrl): State<AppController>,
) -> Result<Json<web_eiou::BalanceResponse>> {
    tracing::debug!("Received eiou balance request");

    let balance = web_eiou::BalanceResponse {
        outstanding: bitcoin::Amount::from_sat(1000),
        treasury: bitcoin::Amount::from_sat(10000),
    };
    Ok(Json(balance))
}
