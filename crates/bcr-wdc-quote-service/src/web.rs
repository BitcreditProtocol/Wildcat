// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_common::{
    core::signature::{deserialize_borsh_msg, schnorr_verify_b64},
    wire::quotes as wire_quotes,
};
// ----- local imports
use crate::{error::Result, quotes, service::Service};

// ----- end imports

///--------------------------- Enquire mint quote
pub async fn enquire_quote(
    State(ctrl): State<Arc<Service>>,
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
fn convert_to_enquire_reply(quote: quotes::Quote) -> wire_quotes::StatusReply {
    match quote.status {
        quotes::Status::Pending { .. } => wire_quotes::StatusReply::Pending,
        quotes::Status::Canceled { tstamp } => wire_quotes::StatusReply::Canceled { tstamp },
        quotes::Status::Denied { tstamp } => wire_quotes::StatusReply::Denied { tstamp },
        quotes::Status::Offered {
            keyset_id,
            ttl,
            discounted,
            wallet_pubkey,
        } => wire_quotes::StatusReply::Offered {
            keyset_id,
            expiration_date: ttl,
            discounted,
            wallet_pubkey,
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
            wallet_pubkey,
        } => wire_quotes::StatusReply::Accepted {
            keyset_id,
            discounted,
            wallet_pubkey,
        },
        quotes::Status::MintingEnabled {
            keyset_id,
            wallet_pubkey,
            discounted,
            ..
        } => wire_quotes::StatusReply::MintingEnabled {
            keyset_id,
            discounted,
            wallet_pubkey,
            minted_amount: bcr_common::cashu::Amount::ZERO,
        },
    }
}

pub async fn lookup_quote(
    State(ctrl): State<Arc<Service>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<wire_quotes::StatusReply>> {
    tracing::debug!("Received mint quote lookup request for id: {}", id);

    let now = chrono::Utc::now();
    let quote = ctrl.lookup(id, now).await?;
    Ok(Json(convert_to_enquire_reply(quote)))
}

/// --------------------------- Resolve quote offer
pub async fn resolve_offer(
    State(ctrl): State<Arc<Service>>,
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
pub async fn cancel(
    State(ctrl): State<Arc<Service>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<wire_quotes::StatusReply>> {
    tracing::debug!("Received mint quote cancel request for id: {}", id);

    let now = chrono::Utc::now();
    ctrl.cancel(id, now).await?;
    let quote = ctrl.lookup(id, now).await?;
    let reply = convert_to_enquire_reply(quote);
    Ok(Json(reply))
}
