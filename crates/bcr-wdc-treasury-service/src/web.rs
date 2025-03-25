// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_wdc_webapi::signatures as web_signatures;
// ----- local imports
use crate::credit;
use crate::debit;
use crate::error::Result;

pub async fn generate_blind_messages<Repo>(
    State(ctrl): State<credit::Service<Repo>>,
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
        rid,
        messages: blinds,
    }))
}

pub async fn store_signatures<Repo>(
    State(ctrl): State<credit::Service<Repo>>,
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

pub async fn request_mint_from_ebill<Wlt>(
    State(ctrl): State<debit::Service<Wlt>>,
    Json(request): Json<web_signatures::RequestToMintFromEBillRequest>,
) -> Result<Json<web_signatures::RequestToMintfromEBillResponse>>
where
    Wlt: debit::Wallet,
{
    let quote = ctrl.mint_from_ebill(request.ebill, request.amount).await?;
    let response = web_signatures::RequestToMintfromEBillResponse {
        id: quote.id,
        request: quote.request,
    };
    Ok(Json(response))
}
