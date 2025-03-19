// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use cashu::nuts::nut03 as cdk03;
// ----- local imports
use crate::error::Result;
use crate::service::Service;
use crate::service::{KeysService, ProofRepository};

pub async fn swap_tokens<KeysSrvc, ProofRepo>(
    State(ctrl): State<Service<KeysSrvc, ProofRepo>>,
    Json(request): Json<cdk03::SwapRequest>,
) -> Result<Json<cdk03::SwapResponse>>
where
    KeysSrvc: KeysService,
    ProofRepo: ProofRepository,
{
    let signatures = ctrl.swap(&request.inputs, &request.outputs).await?;
    let response = cdk03::SwapResponse { signatures };
    Ok(Json(response))
}
