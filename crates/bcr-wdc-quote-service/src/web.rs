// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_wdc_webapi::quotes as web_quotes;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;
// ----- local imports
use crate::error::Result;
use crate::quotes;

///--------------------------- Enquire mint quote
#[utoipa::path(
    post,
    path = "/v1/mint/credit/quote",
    request_body(content = EnquireRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Quote request admitted", body = EnquireReply, content_type = "application/json"),
        (status = 404, description = "Quote request not accepted"),
    )
)]
pub async fn enquire_quote<KG, QR>(
    State(ctrl): State<quotes::Service<KG, QR>>,
    Json(req): Json<web_quotes::EnquireRequest>,
) -> Result<Json<web_quotes::EnquireReply>>
where
    KG: quotes::KeyFactory,
    QR: quotes::Repository,
{
    log::debug!(
        "Received mint quote request for bill: {}, from node : {}",
        req.content.id,
        req.content.holder.name
    );

    verify_signature(&req)?;

    let bcr_wdc_webapi::quotes::EnquireRequest {
        content, outputs, ..
    } = req;
    let bill = quotes::BillInfo::try_from(content)?;
    let id = ctrl.enquire(bill, chrono::Utc::now(), outputs).await?;
    Ok(Json(web_quotes::EnquireReply { id }))
}

fn verify_signature(req: &web_quotes::EnquireRequest) -> Result<()> {
    let author = &req.content.holder;
    let borshed = borsh::to_vec(&req.content)?;
    let msg = bitcoin::secp256k1::Message::from_digest(*Sha256::hash(&borshed).as_byte_array());
    let ctx = bitcoin::secp256k1::Secp256k1::verification_only();
    let pub_key = bitcoin::secp256k1::PublicKey::from_str(&author.node_id)?;
    ctx.verify_schnorr(&req.signature, &msg, &pub_key.x_only_public_key().0)?;
    Ok(())
}

/// --------------------------- Look up quote
fn convert_to_enquire_reply(quote: quotes::Quote) -> web_quotes::StatusReply {
    match quote.status {
        quotes::QuoteStatus::Pending { .. } => web_quotes::StatusReply::Pending,
        quotes::QuoteStatus::Denied => web_quotes::StatusReply::Denied,
        quotes::QuoteStatus::Offered { signatures, ttl } => web_quotes::StatusReply::Offered {
            signatures,
            expiration_date: ttl,
        },
        quotes::QuoteStatus::Rejected { tstamp } => web_quotes::StatusReply::Rejected { tstamp },
        quotes::QuoteStatus::Accepted { signatures } => {
            web_quotes::StatusReply::Accepted { signatures }
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1/mint/credit/quote/{id}",
    params(
        ("id" = Uuid, Path, description = "The quote id")
    ),
    responses (
        (status = 200, description = "Succesful response", body = StatusReply, content_type = "application/json"),
        (status = 404, description = "Quote id not  found"),
    )
)]
pub async fn lookup_quote<KG, QR>(
    State(ctrl): State<quotes::Service<KG, QR>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<web_quotes::StatusReply>>
where
    KG: quotes::KeyFactory,
    QR: quotes::Repository,
{
    log::debug!("Received mint quote lookup request for id: {}", id);

    let quote = ctrl.lookup(id).await?;
    Ok(Json(convert_to_enquire_reply(quote)))
}

#[utoipa::path(
    post,
    path = "/v1/credit/quote/{id}",
    params(
        ("id" = Uuid, Path, description = "The quote id")
    ),
    request_body(content = Resolve, content_type = "application/json"),
    responses (
        (status = 200, description = "Succesful response"),
        (status = 404, description = "Quote not found"),
        (status = 409, description = "Quote already resolved"),
    )
)]
pub async fn resolve_offer<KG, QR>(
    State(ctrl): State<quotes::Service<KG, QR>>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<web_quotes::ResolveOffer>,
) -> Result<()>
where
    KG: quotes::KeyFactory,
    QR: quotes::Repository,
{
    log::debug!("Received mint quote resolve request for id: {}", id);

    match req {
        web_quotes::ResolveOffer::Reject => ctrl.reject(id, chrono::Utc::now()).await?,
        web_quotes::ResolveOffer::Accept => ctrl.accept(id).await?,
    }
    Ok(())
}
