// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Query, State};
use bcr_wdc_webapi::wallet as web_wallet;
use bdk_wallet::bitcoin as btc;
// ----- local imports
use crate::{
    error::Result,
    service::{BalanceRepository, Candle, Service},
};

// ----- end imports

fn convert_candle<Amount, Converter>(
    candle: Candle<Amount>,
    converter: Converter,
) -> web_wallet::Candle
where
    Converter: Fn(Amount) -> u64,
{
    web_wallet::Candle {
        date: candle.tstamp,
        open: converter(candle.open),
        high: converter(candle.high),
        low: converter(candle.low),
        close: converter(candle.close),
    }
}
/// --------------------------- crsat chart
#[utoipa::path(
    get,
    path = "/v1/admin/crsat/chart",
    params(
        ("start" = chrono::NaiveDate, Query, description = "start date for the chart"),
        ("end" = Option<chrono::NaiveDate>, Query, description = "end date for the chart, defaults to today"),
    ),
    responses (
        (status = 200, description = "Successful response", body = web_wallet::CandleChart, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn crsat_chart<DB>(
    State(ctrl): State<Service<DB>>,
    Query(start): Query<chrono::NaiveDate>,
    Query(end): Query<Option<chrono::NaiveDate>>,
) -> Result<Json<web_wallet::CandleChart>>
where
    DB: BalanceRepository + Send + Sync,
{
    tracing::debug!("Received crsat chart request");

    let end = end.unwrap_or(chrono::Utc::now().date_naive());
    let start = start.and_hms_opt(0, 0, 0).unwrap().and_utc();
    let end = end.and_hms_opt(23, 59, 59).unwrap().and_utc();
    let candles = ctrl
        .query_crsat_chart(start, end)
        .await?
        .into_iter()
        .map(|candle| convert_candle(candle, cashu::Amount::into))
        .collect::<Vec<_>>();
    Ok(Json(web_wallet::CandleChart { candles }))
}

/// --------------------------- sat chart
#[utoipa::path(
    get,
    path = "/v1/admin/sat/chart",
    params(
        ("start" = chrono::NaiveDate, Query, description = "start date for the chart"),
        ("end" = Option<chrono::NaiveDate>, Query, description = "end date for the chart, defaults to today"),
    ),
    responses (
        (status = 200, description = "Successful response", body = web_wallet::CandleChart, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn sat_chart<DB>(
    State(ctrl): State<Service<DB>>,
    Query(start): Query<chrono::NaiveDate>,
    Query(end): Query<Option<chrono::NaiveDate>>,
) -> Result<Json<web_wallet::CandleChart>>
where
    DB: BalanceRepository + Send + Sync,
{
    tracing::debug!("Received sat chart request");

    let end = end.unwrap_or(chrono::Utc::now().date_naive());
    let start = start.and_hms_opt(0, 0, 0).unwrap().and_utc();
    let end = end.and_hms_opt(23, 59, 59).unwrap().and_utc();
    let candles = ctrl
        .query_sat_chart(start, end)
        .await?
        .into_iter()
        .map(|candle| convert_candle(candle, cashu::Amount::into))
        .collect::<Vec<_>>();
    Ok(Json(web_wallet::CandleChart { candles }))
}

/// --------------------------- btc onchain chart
#[utoipa::path(
    get,
    path = "/v1/admin/btc/chart",
    params(
        ("start" = chrono::NaiveDate, Query, description = "start date for the chart"),
        ("end" = Option<chrono::NaiveDate>, Query, description = "end date for the chart, defaults to today"),
    ),
    responses (
        (status = 200, description = "Successful response", body = web_wallet::CandleChart, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn btc_chart<DB>(
    State(ctrl): State<Service<DB>>,
    Query(start): Query<chrono::NaiveDate>,
    Query(end): Query<Option<chrono::NaiveDate>>,
) -> Result<Json<web_wallet::CandleChart>>
where
    DB: BalanceRepository + Send + Sync,
{
    tracing::debug!("Received btc chart request");

    let end = end.unwrap_or(chrono::Utc::now().date_naive());
    let start = start.and_hms_opt(0, 0, 0).unwrap().and_utc();
    let end = end.and_hms_opt(23, 59, 59).unwrap().and_utc();
    let candles = ctrl
        .query_onchain_chart(start, end)
        .await?
        .into_iter()
        .map(|candle| convert_candle(candle, btc::Amount::to_sat))
        .collect::<Vec<_>>();
    Ok(Json(web_wallet::CandleChart { candles }))
}

/// --------------------------- e-IOU chart
#[utoipa::path(
    get,
    path = "/v1/admin/eiou/chart",
    params(
        ("start" = chrono::NaiveDate, Query, description = "start date for the chart"),
        ("end" = Option<chrono::NaiveDate>, Query, description = "end date for the chart, defaults to today"),
    ),
    responses (
        (status = 200, description = "Successful response", body = web_wallet::CandleChart, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn eiou_chart<DB>(
    State(ctrl): State<Service<DB>>,
    Query(start): Query<chrono::NaiveDate>,
    Query(end): Query<Option<chrono::NaiveDate>>,
) -> Result<Json<web_wallet::CandleChart>>
where
    DB: BalanceRepository + Send + Sync,
{
    tracing::debug!("Received e-IOU chart request");

    let end = end.unwrap_or(chrono::Utc::now().date_naive());
    let start = start.and_hms_opt(0, 0, 0).unwrap().and_utc();
    let end = end.and_hms_opt(23, 59, 59).unwrap().and_utc();
    let candles = ctrl
        .query_eiou_chart(start, end)
        .await?
        .into_iter()
        .map(|candle| convert_candle(candle, btc::Amount::to_sat))
        .collect::<Vec<_>>();
    Ok(Json(web_wallet::CandleChart { candles }))
}
