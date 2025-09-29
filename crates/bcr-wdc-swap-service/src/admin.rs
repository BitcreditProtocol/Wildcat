// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::wire::swap as wire_swap;
// ----- local imports
use crate::error::Result;
use crate::service::Service;
use crate::service::{KeysService, ProofRepository};

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn recover_tokens<KeysSrvc, ProofRepo>(
    State(ctrl): State<Service<KeysSrvc, ProofRepo>>,
    Json(request): Json<wire_swap::RecoverRequest>,
) -> Result<Json<wire_swap::RecoverResponse>>
where
    KeysSrvc: KeysService,
    ProofRepo: ProofRepository,
{
    ctrl.recover(&request.proofs).await?;
    Ok(Json(wire_swap::RecoverResponse {}))
}
