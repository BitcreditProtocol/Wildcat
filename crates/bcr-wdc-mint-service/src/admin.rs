// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::{
    cashu,
    core::maturity,
    ecash,
    wire::{keys as wire_keys, swap as wire_swap, treasury as wire_treasury},
};
// ----- local imports
use crate::{core::service, error::Result, vault};

// ----- end imports

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn new_keyset(
    State(ctrl): State<Arc<service::Service>>,
    Json(request): Json<wire_keys::NewKeysetRequest>,
) -> Result<Json<ecash::KeySetInfo>> {
    let now = time::OffsetDateTime::now_utc();
    let expiration = request.expiration.map(maturity::credit_expires_at);
    let kinfo = ctrl
        .create(request.unit, now, expiration, request.fees_ppk)
        .await?;
    Ok(Json(kinfo.into()))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn recover_tokens(
    State(ctrl): State<Arc<service::Service>>,
    Json(request): Json<wire_swap::RecoverRequest>,
) -> Result<Json<wire_swap::RecoverResponse>> {
    let c_proofs: Vec<cashu::Proof> = request.proofs.into_iter().map(From::from).collect();
    ctrl.recover(&c_proofs).await?;
    Ok(Json(wire_swap::RecoverResponse {}))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(vault_srvc, core_srvc))]
pub async fn generate_fees_token(
    State(vault_srvc): State<Arc<vault::Service>>,
    State(core_srvc): State<Arc<service::Service>>,
) -> Result<Json<wire_treasury::FeesTokenResponse>> {
    let now = time::OffsetDateTime::now_utc();
    let wdc_cl = vault::WildcatCl { core: core_srvc };
    let token = vault_srvc.generate_token(&wdc_cl, now).await?;
    let total = token.value()?;
    let response = wire_treasury::FeesTokenResponse {
        token: token.to_string(),
        total,
    };
    Ok(Json(response))
}
