// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::{
    cashu,
    core::maturity,
    ecash,
    wire::{keys as wire_keys, swap as wire_swap},
};
use bitcoin::secp256k1 as secp;
// ----- local imports
use crate::{error::Result, service};

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
pub async fn sign_blind(
    State(ctrl): State<Arc<service::Service>>,
    Json(blinds): Json<Vec<cashu::BlindedMessage>>,
) -> Result<Json<Vec<cashu::BlindSignature>>> {
    tracing::debug!("Received sign blind request");

    ctrl.sign_blinds(&blinds).await.map(Json)
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn verify_proof(
    State(ctrl): State<Arc<service::Service>>,
    Json(proof): Json<cashu::Proof>,
) -> Result<()> {
    tracing::debug!("Received verify proof request");

    ctrl.verify_proofs(&[proof]).await
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn verify_fingerprint(
    State(ctrl): State<Arc<service::Service>>,
    Json(fp): Json<wire_keys::ProofFingerprint>,
) -> Result<()> {
    tracing::debug!("Received verify fingerprint request");

    ctrl.verify_fingerprints(&[fp.into()]).await
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

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn burn_tokens(
    State(ctrl): State<Arc<service::Service>>,
    Json(request): Json<wire_swap::BurnRequest>,
) -> Result<Json<wire_swap::BurnResponse>> {
    let wire_swap::BurnRequest { proofs } = request;
    let c_proofs: Vec<cashu::Proof> = proofs.into_iter().map(From::from).collect();
    let c_ys = ctrl.burn(c_proofs).await?;
    let ys = c_ys
        .into_iter()
        .map(|c_y| {
            let b_array = c_y.to_bytes();
            secp::PublicKey::from_slice(&b_array).expect("cashu::PublicKey <-> secp::PublicKey")
        })
        .collect();
    Ok(Json(wire_swap::BurnResponse { ys }))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(swap_srvc))]
pub async fn reserve_ys(
    State(swap_srvc): State<Arc<service::Service>>,
    Json(request): Json<wire_swap::ReserveRequest>,
) -> Result<()> {
    let c_ys = request.ys.into_iter().map(cashu::PublicKey::from).collect();
    swap_srvc.reserve(c_ys, request.deadline).await
}
