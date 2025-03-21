// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_wdc_webapi::swap as web_swap;
// ----- local imports
use crate::error::Result;
use crate::service::Service;
use crate::service::{KeysService, ProofRepository};

pub async fn recover_tokens<KeysSrvc, ProofRepo>(
    State(ctrl): State<Service<KeysSrvc, ProofRepo>>,
    Json(request): Json<web_swap::RecoverRequest>,
) -> Result<Json<web_swap::RecoverResponse>>
where
    KeysSrvc: KeysService,
    ProofRepo: ProofRepository,
{
    ctrl.recover(&request.proofs).await?;
    Ok(Json(web_swap::RecoverResponse {}))
}
