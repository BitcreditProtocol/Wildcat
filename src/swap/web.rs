// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use cdk::nuts::nut03 as cdk03;
// ----- local imports
use crate::swap::error::Result;
use crate::swap::{KeysRepository, ProofRepository, Service};

pub async fn swap_tokens<KR, PR>(
    State(ctrl): State<Service<KR, PR>>,
    Json(req): Json<cdk03::SwapRequest>,
) -> Result<Json<cdk03::SwapResponse>>
where
    KR: KeysRepository,
    PR: ProofRepository,
{
    let signatures = ctrl.swap(&req.inputs, &req.outputs)?;
    let response = cdk03::SwapResponse { signatures };
    Ok(Json(response))
}
