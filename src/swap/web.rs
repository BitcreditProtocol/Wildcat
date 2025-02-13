// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use cdk::nuts::nut03 as cdk03;
// ----- local imports
use crate::swap;
use crate::swap::error::Result;

pub async fn swap_tokens<KR, PR>(
    State(ctrl): State<swap::Service<KR, PR>>,
    Json(request): Json<cdk03::SwapRequest>,
) -> Result<Json<cdk03::SwapResponse>>
where
    KR: swap::KeysRepository,
    PR: swap::ProofRepository,
{
    let signatures = ctrl.swap(&request.inputs, &request.outputs).await?;
    let response = cdk03::SwapResponse { signatures };
    Ok(Json(response))
}
