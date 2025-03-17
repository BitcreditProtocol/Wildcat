// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_wdc_webapi::signatures as web_signatures;
// ----- local imports
use crate::error::Result;
use crate::service::{Repository, Service};

pub async fn generate_blind_messages<Repo>(
    State(ctrl): State<Service<Repo>>,
    Json(request): Json<web_signatures::GenerateBlindedMessagesRequest>,
) -> Result<Json<web_signatures::GenerateBlindedMessagesResponse>>
where
    Repo: Repository,
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
    State(ctrl): State<Service<Repo>>,
    Json(request): Json<web_signatures::StoreBlindedSignaturesRequest>,
) -> Result<()>
where
    Repo: Repository,
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
