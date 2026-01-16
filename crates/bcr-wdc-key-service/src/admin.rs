// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_common::wire::keys as wire_keys;
// ----- local imports
use crate::{error::Result, service};

// ----- end imports

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn sign_blind(
    State(ctrl): State<service::Service>,
    Json(blind): Json<cashu::BlindedMessage>,
) -> Result<Json<cashu::BlindSignature>> {
    tracing::debug!("Received sign blind request");

    ctrl.sign_blind(&blind).await.map(Json)
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn verify_proof(
    State(ctrl): State<service::Service>,
    Json(proof): Json<cashu::Proof>,
) -> Result<()> {
    tracing::debug!("Received verify proof request");

    ctrl.verify_proof(proof).await
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn verify_fingerprint(
    State(ctrl): State<service::Service>,
    Json(fp): Json<wire_keys::ProofFingerprint>,
) -> Result<()> {
    tracing::debug!("Received verify fingerprint request");

    ctrl.verify_fingerprint(fp.into()).await
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_keyset_for_date(
    State(ctrl): State<service::Service>,
    Path(date): Path<chrono::NaiveDate>,
) -> Result<Json<cashu::Id>> {
    tracing::debug!("Received get_keyset_for_date request");

    let kid = ctrl.get_keyset_id_for_date(date).await?;
    Ok(Json(kid))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn deactivate(
    State(ctrl): State<service::Service>,
    Json(request): Json<wire_keys::DeactivateKeysetRequest>,
) -> Result<Json<wire_keys::DeactivateKeysetResponse>> {
    tracing::debug!("Received deactivate request");

    let kid = ctrl.deactivate(request.kid).await?;
    let response = wire_keys::DeactivateKeysetResponse { kid };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn new_mintop(
    State(ctrl): State<service::Service>,
    Json(request): Json<wire_keys::NewMintOperationRequest>,
) -> Result<Json<wire_keys::NewMintOperationResponse>> {
    tracing::debug!("Received new mint operation request");

    ctrl.new_minting_operation(
        request.quote_id,
        request.kid,
        request.pub_key,
        request.target,
        request.bill_id,
    )
    .await?;
    let response = wire_keys::NewMintOperationResponse {};
    Ok(Json(response))
}

fn convert_mintop_status(status: service::MintOperation) -> wire_keys::MintOperationStatus {
    wire_keys::MintOperationStatus {
        kid: status.kid,
        quote_id: status.uid,
        target: status.target,
        current: status.minted,
    }
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn mintop_status(
    State(ctrl): State<service::Service>,
    Path(qid): Path<uuid::Uuid>,
) -> Result<Json<wire_keys::MintOperationStatus>> {
    tracing::debug!("Received mint operation status request {qid}");

    let status = ctrl.mintop_status(qid).await?;
    let status = convert_mintop_status(status);
    Ok(Json(status))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_mintops(
    State(ctrl): State<service::Service>,
    Path(kid): Path<cashu::Id>,
) -> Result<Json<Vec<cashu::Amount>>> {
    tracing::debug!("Received list mint operations request");

    let mint_ops = ctrl.list_mintops_for_kid(kid).await?;
    let response = mint_ops.into_iter().map(|mop| mop.minted).collect();
    Ok(Json(response))
}
