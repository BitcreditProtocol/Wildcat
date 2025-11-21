// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_common::{
    core::signature::{deserialize_borsh_msg, schnorr_verify_b64},
    wire::quotes as wire_quotes,
};
// ----- local imports
use crate::{
    error::Result,
    quotes,
    service::{self, Service},
};

// ----- end imports

///--------------------------- Enquire mint quote
#[utoipa::path(
    post,
    path = "/v1/mint/credit/quote",
    request_body(content = wire_quotes::EnquireRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Quote request admitted", body = wire_quotes::EnquireReply, content_type = "application/json"),
        (status = 404, description = "Quote request not accepted"),
    )
)]
pub async fn enquire_quote(
    State(ctrl): State<Service>,
    Json(signed_request): Json<wire_quotes::SignedEnquireRequest>,
) -> Result<Json<wire_quotes::EnquireReply>> {
    tracing::debug!("Received mint quote request for bill",);

    let payload: wire_quotes::EnquireRequest = deserialize_borsh_msg(&signed_request.content)?;
    let bill_info = ctrl
        .validate_and_decrypt_shared_bill(&payload.content)
        .await?;
    // after validating bill, validate req using the calculated holder
    let holder = bill_info.endorsees.last().unwrap_or(&bill_info.payee);
    schnorr_verify_b64(
        &signed_request.content,
        &signed_request.signature,
        &holder.node_id().pub_key().x_only_public_key().0,
    )?;
    let bill = quotes::convert_to_billinfo(bill_info, payload.content)?;
    let id = ctrl
        .enquire(bill, payload.minting_pubkey, chrono::Utc::now())
        .await?;
    Ok(Json(wire_quotes::EnquireReply { id }))
}

/// --------------------------- Look up quote
fn convert_mint_status(status: service::MintingStatus) -> wire_quotes::MintingStatus {
    match status {
        service::MintingStatus::Disabled => wire_quotes::MintingStatus::Disabled,
        service::MintingStatus::Enabled(minted) => wire_quotes::MintingStatus::Enabled { minted },
    }
}
fn convert_to_enquire_reply(
    quote: quotes::Quote,
    minting_status: service::MintingStatus,
) -> wire_quotes::StatusReply {
    match quote.status {
        quotes::Status::Pending { .. } => wire_quotes::StatusReply::Pending,
        quotes::Status::Canceled { tstamp } => wire_quotes::StatusReply::Canceled { tstamp },
        quotes::Status::Denied { tstamp } => wire_quotes::StatusReply::Denied { tstamp },
        quotes::Status::Offered {
            keyset_id,
            ttl,
            discounted,
            minting_pubkey,
        } => wire_quotes::StatusReply::Offered {
            keyset_id,
            expiration_date: ttl,
            discounted,
            minting_pubkey,
        },
        quotes::Status::OfferExpired { tstamp, discounted } => {
            wire_quotes::StatusReply::OfferExpired { tstamp, discounted }
        }
        quotes::Status::Rejected { tstamp, discounted } => {
            wire_quotes::StatusReply::Rejected { tstamp, discounted }
        }
        quotes::Status::Accepted {
            keyset_id,
            discounted,
            minting_pubkey,
        } => wire_quotes::StatusReply::Accepted {
            keyset_id,
            discounted,
            minting_pubkey,
            minting_status: convert_mint_status(minting_status),
        },
    }
}

#[utoipa::path(
    get,
    path = "/v1/mint/credit/quote/{id}",
    params(
        ("id" = Uuid, Path, description = "The quote id")
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_quotes::StatusReply, content_type = "application/json"),
        (status = 404, description = "Quote id not  found"),
    )
)]
pub async fn lookup_quote(
    State(ctrl): State<Service>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<wire_quotes::StatusReply>> {
    tracing::debug!("Received mint quote lookup request for id: {}", id);

    let now = chrono::Utc::now();
    let quote = ctrl.lookup(id, now).await?;
    let mint_status = ctrl.minting_status(id).await?;
    Ok(Json(convert_to_enquire_reply(quote, mint_status)))
}

/// --------------------------- Resolve quote offer
#[utoipa::path(
    post,
    path = "/v1/mint/credit/quote/{id}",
    params(
        ("id" = Uuid, Path, description = "The quote id")
    ),
    request_body(content = wire_quotes::ResolveOffer, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response"),
        (status = 404, description = "Quote not found"),
        (status = 409, description = "Quote already resolved"),
    )
)]
pub async fn resolve_offer(
    State(ctrl): State<Service>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<wire_quotes::ResolveOffer>,
) -> Result<()> {
    tracing::debug!("Received mint quote resolve request for id: {}", id);

    let now = chrono::Utc::now();
    match req {
        wire_quotes::ResolveOffer::Reject => ctrl.reject(id, now).await?,
        wire_quotes::ResolveOffer::Accept => ctrl.accept(id, now).await?,
    }
    Ok(())
}

/// --------------------------- Cancel quote inquiry
#[utoipa::path(
    delete,
    path = "/v1/credit/quote/{id}",
    params(
        ("id" = Uuid, Path, description = "The quote id")
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_quotes::StatusReply, content_type = "application/json"),
        (status = 404, description = "Quote not found"),
        (status = 409, description = "Quote already resolved"),
    )
)]
pub async fn cancel(
    State(ctrl): State<Service>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<wire_quotes::StatusReply>> {
    tracing::debug!("Received mint quote cancel request for id: {}", id);

    let now = chrono::Utc::now();
    ctrl.cancel(id, now).await?;
    let quote = ctrl.lookup(id, now).await?;
    let status = ctrl.minting_status(id).await?;
    let reply = convert_to_enquire_reply(quote, status);
    Ok(Json(reply))
}
