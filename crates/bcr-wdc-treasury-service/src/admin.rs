// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::wire::signatures as wire_signatures;
use bcr_wdc_webapi::{signatures as web_signatures, wallet as web_wallet};
use cashu::{self as cdk};
// ----- local imports
use crate::credit;
use crate::debit;
use crate::error::Result;

// ----- end imports

// ----- crsat APIs
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn generate_blinds<Repo, KeySrvc>(
    State(ctrl): State<credit::Service<Repo, KeySrvc>>,
    Json(request): Json<web_signatures::GenerateBlindedMessagesRequest>,
) -> Result<Json<web_signatures::GenerateBlindedMessagesResponse>>
where
    Repo: credit::Repository,
{
    tracing::debug!("Received request to generate blinds",);

    let (rid, blinds) = ctrl.generate_blinds(request.kid, request.total).await?;
    Ok(Json(web_signatures::GenerateBlindedMessagesResponse {
        request_id: rid,
        messages: blinds,
    }))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn store_signatures<Repo, KeySrvc>(
    State(ctrl): State<credit::Service<Repo, KeySrvc>>,
    Json(request): Json<web_signatures::StoreBlindSignaturesRequest>,
) -> Result<()>
where
    Repo: credit::Repository,
{
    tracing::debug!("Received request to store signatures",);

    ctrl.store_signatures(request.rid, request.signatures)
        .await?;
    Ok(())
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn crsat_balance<Repo, KeySrvc>(
    State(ctrl): State<credit::Service<Repo, KeySrvc>>,
) -> Result<Json<web_wallet::ECashBalance>>
where
    Repo: credit::Repository,
    KeySrvc: credit::KeyService,
{
    tracing::debug!("Received request to crsat_balance");

    let amount = ctrl.balance().await?;
    let response = web_wallet::ECashBalance {
        amount,
        unit: cdk::CurrencyUnit::Custom("crsat".to_string()),
    };
    Ok(Json(response))
}

// ----- sat APIs
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn request_mint_from_ebill<Wlt, WdcSrvc, Repo>(
    State(ctrl): State<debit::Service<Wlt, WdcSrvc, Repo>>,
    Json(request): Json<wire_signatures::RequestToMintFromEBillRequest>,
) -> Result<Json<wire_signatures::RequestToMintFromEBillResponse>>
where
    Wlt: debit::Wallet + 'static,
    WdcSrvc: debit::WildcatService + 'static,
    Repo: debit::Repository + 'static,
{
    tracing::debug!("Received request to mint from ebill");

    //TODO! wait for bitcredit-core to integrate bcr-common
    let ebill_id = bcr_ebill_core::bill::BillId::from_str(&request.ebill_id.to_string())
        .expect("compatible billID");
    let quote = ctrl.mint_from_ebill(ebill_id, request.amount).await?;
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
