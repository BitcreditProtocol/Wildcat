// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_wdc_webapi::quotes as web_quotes;
// ----- local imports
use crate::error::Result;
use crate::{
    quotes,
    service::{KeysHandler, Repository, Service, Wallet},
};

///--------------------------- Enquire mint quote
#[utoipa::path(
    post,
    path = "/v1/mint/credit/quote",
    request_body(content = web_quotes::EnquireRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Quote request admitted", body = web_quotes::EnquireReply, content_type = "application/json"),
        (status = 404, description = "Quote request not accepted"),
    )
)]
pub async fn enquire_quote<KeysHndlr, Wlt, QuotesRepo>(
    State(ctrl): State<Service<KeysHndlr, Wlt, QuotesRepo>>,
    Json(req): Json<web_quotes::SignedEnquireRequest>,
) -> Result<Json<web_quotes::EnquireReply>>
where
    QuotesRepo: Repository,
{
    tracing::debug!(
        "Received mint quote request for bill: {}",
        req.request.content.id,
    );

    verify_signature(&req)?;

    let bcr_wdc_webapi::quotes::EnquireRequest {
        content,
        public_key,
    } = req.request;
    let bill = quotes::BillInfo::try_from(content)?;
    let id = ctrl.enquire(bill, public_key, chrono::Utc::now()).await?;
    Ok(Json(web_quotes::EnquireReply { id }))
}

fn verify_signature(req: &web_quotes::SignedEnquireRequest) -> Result<()> {
    let holder = req
        .request
        .content
        .endorsees
        .last()
        .unwrap_or(&req.request.content.payee);
    let pub_key = holder.node_id().pub_key();
    bcr_wdc_utils::keys::schnorr_verify_borsh_msg_with_key(
        &req.request,
        &req.signature,
        &pub_key.x_only_public_key().0,
    )?;
    Ok(())
}

/// --------------------------- Look up quote
fn convert_to_enquire_reply(quote: quotes::Quote) -> web_quotes::StatusReply {
    match quote.status {
        quotes::Status::Pending { .. } => web_quotes::StatusReply::Pending,
        quotes::Status::Canceled { tstamp } => web_quotes::StatusReply::Canceled { tstamp },
        quotes::Status::Denied { tstamp } => web_quotes::StatusReply::Denied { tstamp },
        quotes::Status::Offered {
            keyset_id,
            ttl,
            discounted,
        } => web_quotes::StatusReply::Offered {
            keyset_id,
            expiration_date: ttl,
            discounted,
        },
        quotes::Status::OfferExpired { tstamp, discounted } => {
            web_quotes::StatusReply::OfferExpired { tstamp, discounted }
        }
        quotes::Status::Rejected { tstamp, discounted } => {
            web_quotes::StatusReply::Rejected { tstamp, discounted }
        }
        quotes::Status::Accepted {
            keyset_id,
            discounted,
        } => web_quotes::StatusReply::Accepted {
            keyset_id,
            discounted,
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
        (status = 200, description = "Successful response", body = web_quotes::StatusReply, content_type = "application/json"),
        (status = 404, description = "Quote id not  found"),
    )
)]
pub async fn lookup_quote<KeysHndlr, Wlt, QuotesRepo>(
    State(ctrl): State<Service<KeysHndlr, Wlt, QuotesRepo>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<web_quotes::StatusReply>>
where
    KeysHndlr: KeysHandler,
    Wlt: Wallet,
    QuotesRepo: Repository,
{
    tracing::debug!("Received mint quote lookup request for id: {}", id);

    let now = chrono::Utc::now();
    let quote = ctrl.lookup(id, now).await?;
    Ok(Json(convert_to_enquire_reply(quote)))
}

/// --------------------------- Resolve quote offer
#[utoipa::path(
    post,
    path = "/v1/mint/credit/quote/{id}",
    params(
        ("id" = Uuid, Path, description = "The quote id")
    ),
    request_body(content = web_quotes::ResolveOffer, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response"),
        (status = 404, description = "Quote not found"),
        (status = 409, description = "Quote already resolved"),
    )
)]
pub async fn resolve_offer<KeysHndlr, Wlt, QuotesRepo>(
    State(ctrl): State<Service<KeysHndlr, Wlt, QuotesRepo>>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<web_quotes::ResolveOffer>,
) -> Result<()>
where
    KeysHndlr: KeysHandler,
    Wlt: Wallet,
    QuotesRepo: Repository,
{
    tracing::debug!("Received mint quote resolve request for id: {}", id);

    let now = chrono::Utc::now();
    match req {
        web_quotes::ResolveOffer::Reject => ctrl.reject(id, now).await?,
        web_quotes::ResolveOffer::Accept => ctrl.accept(id, now).await?,
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
        (status = 200, description = "Successful response", body = web_quotes::StatusReply, content_type = "application/json"),
        (status = 404, description = "Quote not found"),
        (status = 409, description = "Quote already resolved"),
    )
)]
pub async fn cancel<KeysHndlr, Wlt, QuotesRepo>(
    State(ctrl): State<Service<KeysHndlr, Wlt, QuotesRepo>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<web_quotes::StatusReply>>
where
    KeysHndlr: KeysHandler,
    Wlt: Wallet,
    QuotesRepo: Repository,
{
    tracing::debug!("Received mint quote cancel request for id: {}", id);

    let now = chrono::Utc::now();
    ctrl.cancel(id, now).await?;
    let quote = ctrl.lookup(id, now).await?;
    let reply = convert_to_enquire_reply(quote);
    Ok(Json(reply))
}
