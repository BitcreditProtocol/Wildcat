// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, Query, State};
use bcr_wdc_webapi::quotes as web_quotes;
// ----- local imports
use crate::credit::error::Result;
use crate::credit::quotes;
use crate::utils;

/// --------------------------- List quotes
#[utoipa::path(
    get,
    path = "/v1/admin/credit/quote/pending",
    params(
        ("since" = Option<chrono::NaiveDateTime>, Query, description = "only quote requests younger than `since`")
    ),
    responses (
        (status = 200, description = "Succesful response", body = ListReply, content_type = "application/json"),
    )
)]
pub async fn list_pending_quotes<KG, QR>(
    State(ctrl): State<quotes::Service<KG, QR>>,
    since: Option<Query<chrono::NaiveDateTime>>,
) -> Result<Json<web_quotes::ListReply>>
where
    KG: quotes::KeyFactory,
    QR: quotes::Repository,
{
    log::debug!("Received request to list pending quotes");

    let quotes = ctrl.list_pendings(since.map(|q| q.0.and_utc())).await?;
    Ok(Json(web_quotes::ListReply { quotes }))
}

#[utoipa::path(
    get,
    path = "/v1/admin/credit/quote/accepted",
    params(
        ("since" = Option<chrono::NaiveDateTime>, Query, description = "only accepted quotes younger than `since`")
    ),
    responses (
        (status = 200, description = "Succesful response", body = ListReply, content_type = "application/json"),
    )
)]
pub async fn list_accepted_quotes<KG, QR>(
    State(ctrl): State<quotes::Service<KG, QR>>,
    since: Option<Query<chrono::NaiveDateTime>>,
) -> Result<Json<web_quotes::ListReply>>
where
    KG: quotes::KeyFactory,
    QR: quotes::Repository,
{
    log::debug!("Received request to list accepted quotes");

    let quotes = ctrl.list_accepteds(since.map(|q| q.0.and_utc())).await?;
    Ok(Json(web_quotes::ListReply { quotes }))
}

/// --------------------------- Look up request
fn convert_to_info_reply(quote: quotes::Quote) -> web_quotes::InfoReply {
    match quote.status {
        quotes::QuoteStatus::Pending { .. } => web_quotes::InfoReply::Pending {
            id: quote.id,
            bill: quote.bill,
            endorser: quote.endorser,
            submitted: quote.submitted,
            suggested_expiration: utils::calculate_default_expiration_date_for_quote(
                chrono::Utc::now(),
            ),
        },
        quotes::QuoteStatus::Accepted { signatures, ttl } => web_quotes::InfoReply::Accepted {
            id: quote.id,
            bill: quote.bill.clone(),
            endorser: quote.endorser.clone(),
            ttl,
            signatures: signatures.clone(),
        },
        quotes::QuoteStatus::Declined => web_quotes::InfoReply::Declined {
            id: quote.id,
            bill: quote.bill,
            endorser: quote.endorser,
        },
    }
}

#[utoipa::path(
    get,
    path = "/v1/admin/credit/quote/:id",
    params(
        ("id" = String, Path, description = "The quote id")
    ),
    responses (
        (status = 200, description = "Succesful response", body = InfoReply, content_type = "application/json"),
        (status = 404, description = "Quote id not  found"),
    )
)]
pub async fn lookup_quote<KG, QR>(
    State(ctrl): State<quotes::Service<KG, QR>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<web_quotes::InfoReply>>
where
    KG: quotes::KeyFactory,
    QR: quotes::Repository,
{
    log::debug!("Received mint quote lookup request for id: {}", id);

    let quote = ctrl.lookup(id).await?;
    let response = convert_to_info_reply(quote);
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/v1/admin/credit/quote/:id",
    params(
        ("id" = String, Path, description = "The quote id")
    ),
    request_body(content = ResolveRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Succesful response"),
    )
)]
pub async fn resolve_quote<KG, QR>(
    State(ctrl): State<quotes::Service<KG, QR>>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<web_quotes::ResolveRequest>,
) -> Result<()>
where
    KG: quotes::KeyFactory,
    QR: quotes::Repository,
{
    log::debug!("Received mint quote resolve request for id: {}", id);

    match req {
        web_quotes::ResolveRequest::Decline => ctrl.decline(id).await?,
        web_quotes::ResolveRequest::Accept { discount, ttl } => {
            ctrl.accept(id, discount, chrono::Utc::now(), ttl).await?
        }
    }
    Ok(())
}
