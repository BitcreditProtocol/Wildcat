// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_wdc_webapi::{signatures as web_signatures, wallet as web_wallet};
use cashu::{self as cdk, nut03 as cdk03};
// ----- local imports
use crate::credit;
use crate::debit;
use crate::error::Result;

// ----- crsat APIs
pub async fn generate_blind_messages<Repo, KeySrvc>(
    State(ctrl): State<credit::Service<Repo, KeySrvc>>,
    Json(request): Json<web_signatures::GenerateBlindedMessagesRequest>,
) -> Result<Json<web_signatures::GenerateBlindedMessagesResponse>>
where
    Repo: credit::Repository,
{
    log::debug!(
        "Received request to generate blinds for {} id: {}",
        request.total,
        request.kid
    );

    let (rid, blinds) = ctrl.generate_blinds(request.kid, request.total).await?;
    Ok(Json(web_signatures::GenerateBlindedMessagesResponse {
        request_id: rid,
        messages: blinds,
    }))
}

pub async fn store_signatures<Repo, KeySrvc>(
    State(ctrl): State<credit::Service<Repo, KeySrvc>>,
    Json(request): Json<web_signatures::StoreBlindSignaturesRequest>,
) -> Result<()>
where
    Repo: credit::Repository,
{
    log::debug!(
        "Received request to store {} signatures rid: {}",
        request.signatures.len(),
        request.rid,
    );

    ctrl.store_signatures(request.rid, request.signatures, request.expiration)
        .await?;
    Ok(())
}

pub async fn crsat_balance<Repo, KeySrvc>(
    State(ctrl): State<credit::Service<Repo, KeySrvc>>,
) -> Result<Json<web_wallet::ECashBalance>>
where
    Repo: credit::Repository,
    KeySrvc: credit::KeyService,
{
    log::debug!("Received request to crsat_balance");

    let amount = ctrl.balance().await?;
    let response = web_wallet::ECashBalance {
        amount,
        unit: cdk::CurrencyUnit::Custom("crsat".to_string()),
    };
    Ok(Json(response))
}

// ----- sat APIs
pub async fn request_mint_from_ebill<Wlt, ProofCl>(
    State(ctrl): State<debit::Service<Wlt, ProofCl>>,
    Json(request): Json<web_signatures::RequestToMintFromEBillRequest>,
) -> Result<Json<web_signatures::RequestToMintfromEBillResponse>>
where
    Wlt: debit::Wallet,
{
    log::debug!("Received request to mint from ebill {}", request.ebill_id);

    let quote = ctrl
        .mint_from_ebill(request.ebill_id, request.amount)
        .await?;
    let response = web_signatures::RequestToMintfromEBillResponse {
        request_id: quote.id,
        request: quote.request,
    };
    Ok(Json(response))
}

pub async fn redeem<Wlt, ProofCl>(
    State(ctrl): State<debit::Service<Wlt, ProofCl>>,
    Json(request): Json<cdk03::SwapRequest>,
) -> Result<Json<cdk03::SwapResponse>>
where
    Wlt: debit::Wallet,
    ProofCl: debit::ProofClient,
{
    log::debug!(
        "Received request to redeem {} inputs",
        request.inputs().len()
    );

    let signatures = ctrl.redeem(request.inputs(), request.outputs()).await?;
    let response = cdk03::SwapResponse { signatures };
    Ok(Json(response))
}
