// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_web_api::quotes as web_quotes;
// ----- local imports
use crate::credit::error::Result;
use crate::credit::quotes;

///--------------------------- Enquire mint quote
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
        req.bill,
        req.node
    );

    let id = ctrl
        .enquire(req.bill, req.node, chrono::Utc::now(), req.outputs)
        .await?;
    Ok(Json(web_quotes::EnquireReply { id }))
}

/// --------------------------- Look up quote
fn convert_to_enquire_reply(quote: quotes::Quote) -> web_quotes::StatusReply {
    match quote.status {
        quotes::QuoteStatus::Pending { .. } => web_quotes::StatusReply::Pending,
        quotes::QuoteStatus::Declined => web_quotes::StatusReply::Declined,
        quotes::QuoteStatus::Accepted { signatures, ttl } => web_quotes::StatusReply::Accepted {
            signatures,
            expiration_date: ttl,
        },
    }
}

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
