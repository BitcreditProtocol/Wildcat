// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_wdc_webapi::swap as web_swap;
use cashu::{nut03 as cdk03, nut07 as cdk07};
// ----- local imports
use crate::error::Result;
use crate::service::Service;
use crate::service::{KeysService, ProofRepository};

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn swap_tokens<KeysSrvc, ProofRepo>(
    State(ctrl): State<Service<KeysSrvc, ProofRepo>>,
    Json(request): Json<cdk03::SwapRequest>,
) -> Result<Json<cdk03::SwapResponse>>
where
    KeysSrvc: KeysService,
    ProofRepo: ProofRepository,
{
    let signatures = ctrl.swap(request.inputs(), request.outputs()).await?;
    let response = cdk03::SwapResponse { signatures };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn burn_tokens<KeysSrvc, ProofRepo>(
    State(ctrl): State<Service<KeysSrvc, ProofRepo>>,
    Json(request): Json<web_swap::BurnRequest>,
) -> Result<Json<web_swap::BurnResponse>>
where
    KeysSrvc: KeysService,
    ProofRepo: ProofRepository,
{
    let ys = ctrl.burn(&request.proofs).await?;
    Ok(Json(web_swap::BurnResponse { ys }))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn check_state<KeysSrvc, ProofRepo>(
    State(ctrl): State<Service<KeysSrvc, ProofRepo>>,
    Json(request): Json<cdk07::CheckStateRequest>,
) -> Result<Json<cdk07::CheckStateResponse>>
where
    ProofRepo: ProofRepository,
{
    let states = ctrl.check_spendable(&request.ys).await?;
    let response = cdk07::CheckStateResponse { states };
    Ok(Json(response))
}
