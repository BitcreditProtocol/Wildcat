// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::{
    cashu, cdk_common,
    wire::{keys as wire_keys, swap as wire_swap},
};
// ----- local imports
use crate::{error::Result, keys, swap};

// ----- end imports

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn new_keyset(
    State(ctrl): State<Arc<keys::service::Service>>,
    Json(request): Json<wire_keys::NewKeysetRequest>,
) -> Result<Json<cdk_common::mint::MintKeySetInfo>> {
    tracing::debug!("Received new keyset request");

    let now = chrono::Utc::now();
    let expiration = request
        .expiration
        .map(|date| date.and_time(chrono::NaiveTime::MIN).and_utc());
    let kinfo = ctrl
        .create(request.unit, now, expiration, request.fees_ppk)
        .await?;
    Ok(Json(kinfo))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn sign_blind(
    State(ctrl): State<Arc<keys::service::Service>>,
    Json(blinds): Json<Vec<cashu::BlindedMessage>>,
) -> Result<Json<Vec<cashu::BlindSignature>>> {
    tracing::debug!("Received sign blind request");

    ctrl.sign_blinds(blinds.iter()).await.map(Json)
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn verify_proof(
    State(ctrl): State<Arc<keys::service::Service>>,
    Json(proof): Json<cashu::Proof>,
) -> Result<()> {
    tracing::debug!("Received verify proof request");

    ctrl.verify_proofs(&[proof]).await
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn verify_fingerprint(
    State(ctrl): State<Arc<keys::service::Service>>,
    Json(fp): Json<wire_keys::ProofFingerprint>,
) -> Result<()> {
    tracing::debug!("Received verify fingerprint request");

    ctrl.verify_fingerprints(&[fp.into()]).await
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn deactivate(
    State(ctrl): State<Arc<keys::service::Service>>,
    Json(request): Json<wire_keys::DeactivateKeysetRequest>,
) -> Result<Json<wire_keys::DeactivateKeysetResponse>> {
    tracing::debug!("Received deactivate request");

    let kid = ctrl.deactivate(request.kid).await?;
    let response = wire_keys::DeactivateKeysetResponse { kid };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn recover_tokens(
    State(ctrl): State<Arc<swap::service::Service>>,
    Json(request): Json<wire_swap::RecoverRequest>,
) -> Result<Json<wire_swap::RecoverResponse>> {
    ctrl.recover(&request.proofs).await?;
    Ok(Json(wire_swap::RecoverResponse {}))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, keys_srvc))]
pub async fn burn_tokens(
    State(ctrl): State<Arc<swap::service::Service>>,
    State(keys_srvc): State<Arc<keys::service::Service>>,
    Json(request): Json<wire_swap::BurnRequest>,
) -> Result<Json<wire_swap::BurnResponse>> {
    let signsrvc = swap::KeysSignService { keys: keys_srvc };
    let ys = ctrl.burn(&signsrvc, &request.proofs).await?;
    Ok(Json(wire_swap::BurnResponse { ys }))
}
