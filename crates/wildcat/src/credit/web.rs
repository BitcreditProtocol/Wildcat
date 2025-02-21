// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_wdc_webapi::quotes as web_quotes;
// ----- local imports
use crate::credit::error::Result;
use crate::credit::quotes;

///--------------------------- Enquire mint quote
#[utoipa::path(
    post,
    path = "/v1/credit/mint/quote",
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
    path = "/v1/credit/mint/quote/:id",
    params(
        ("id" = String, Path, description = "The quote id")
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
    path = "/v1/credit/quote/:id",
    params(
        ("id" = String, Path, description = "The quote id")
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
