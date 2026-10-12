// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_common::{
    cashu,
    client::ebill::Client as EbillClient,
    core::maturity,
    ecash,
    wire::{keys as wire_keys, swap as wire_swap, treasury as wire_treasury},
};
// ----- local imports
use crate::{core, ebill, error::Result, vault};

// ----- end imports

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn new_keyset(
    State(ctrl): State<Arc<core::Service>>,
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
    State(ctrl): State<Arc<core::Service>>,
    Json(request): Json<wire_swap::RecoverRequest>,
) -> Result<Json<wire_swap::RecoverResponse>> {
    let c_proofs: Vec<cashu::Proof> = request.proofs.into_iter().map(From::from).collect();
    ctrl.recover(&c_proofs).await?;
    Ok(Json(wire_swap::RecoverResponse {}))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(vault_srvc, core_srvc))]
pub async fn generate_fees_token(
    State(vault_srvc): State<Arc<vault::Service>>,
    State(core_srvc): State<Arc<core::Service>>,
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

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, core_srvc, ebill_cl))]
pub async fn request_to_pay_ebill(
    State(ctrl): State<Arc<ebill::Service>>,
    State(core_srvc): State<Arc<core::Service>>,
    State(ebill_cl): State<Arc<EbillClient>>,
    Json(request): Json<wire_treasury::RequestToPayFromEBillRequest>,
) -> Result<Json<wire_treasury::RequestToPayFromEBillResponse>> {
    let wdc_cl = ebill::WildcatCl {
        core: core_srvc,
        ebill: ebill_cl,
    };
    ctrl.request_to_pay_ebill(&wdc_cl, request.ebill_id, request.amount, request.deadline)
        .await?;
    let response = wire_treasury::RequestToPayFromEBillResponse {};
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, core_srvc, ebill_cl))]
pub async fn new_ebill_mintop(
    State(ctrl): State<Arc<ebill::Service>>,
    State(core_srvc): State<Arc<core::Service>>,
    State(ebill_cl): State<Arc<EbillClient>>,
    Json(request): Json<wire_treasury::NewMintOperationRequest>,
) -> Result<Json<wire_treasury::NewMintOperationResponse>> {
    let now = time::OffsetDateTime::now_utc();
    let wdc_cl = ebill::WildcatCl {
        core: core_srvc,
        ebill: ebill_cl,
    };
    ctrl.new_minting_operation(
        &wdc_cl,
        request.quote_id,
        request.kid.into(),
        cashu::PublicKey::from(request.pub_key),
        request.target,
        request.bill_id,
        now,
    )
    .await?;
    let response = wire_treasury::NewMintOperationResponse {};
    Ok(Json(response))
}

fn convert_ebill_mintop_status(status: ebill::MintOperation) -> wire_treasury::MintOperationStatus {
    wire_treasury::MintOperationStatus {
        kid: status.kid.into(),
        quote_id: status.uid,
        target: status.target,
        current: status.minted,
    }
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn ebill_mintop_status(
    State(ctrl): State<Arc<ebill::Service>>,
    Path(qid): Path<uuid::Uuid>,
) -> Result<Json<wire_treasury::MintOperationStatus>> {
    let status = ctrl.mintop_status(qid).await?;
    let status = convert_ebill_mintop_status(status);
    Ok(Json(status))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_ebill_mintops(
    State(ctrl): State<Arc<ebill::Service>>,
    Path(kid): Path<cashu::Id>,
) -> Result<Json<Vec<uuid::Uuid>>> {
    let mint_ops = ctrl.list_mintops_for_kid(kid).await?;
    let response = mint_ops.into_iter().map(|mop| mop.uid).collect();
    Ok(Json(response))
}
