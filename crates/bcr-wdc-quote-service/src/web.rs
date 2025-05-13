// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_wdc_webapi::{bill::NodeId, quotes as web_quotes};
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
    Json(req): Json<web_quotes::EnquireRequest>,
) -> Result<Json<web_quotes::EnquireReply>>
where
    QuotesRepo: Repository,
{
    tracing::debug!("Received mint quote request for bill: {}", req.content.id,);

    verify_signature(&req)?;

    let bcr_wdc_webapi::quotes::EnquireRequest {
        content,
        public_key,
        ..
    } = req;
    let bill = quotes::BillInfo::try_from(content)?;
    let id = ctrl.enquire(bill, public_key, chrono::Utc::now()).await?;
    Ok(Json(web_quotes::EnquireReply { id }))
}

fn verify_signature(req: &web_quotes::EnquireRequest) -> Result<()> {
    let holder = req.content.endorsees.last().unwrap_or(&req.content.payee);
    let pub_key = bitcoin::secp256k1::PublicKey::from_str(&holder.node_id())?;
    bcr_wdc_utils::keys::schnorr_verify_borsh_msg_with_key(
        &req.content,
        &req.signature,
        &pub_key.x_only_public_key().0,
    )?;
    Ok(())
}

/// --------------------------- Look up quote
fn convert_to_enquire_reply(quote: quotes::Quote) -> web_quotes::StatusReply {
    match quote.status {
        quotes::QuoteStatus::Pending { .. } => web_quotes::StatusReply::Pending,
        quotes::QuoteStatus::Denied => web_quotes::StatusReply::Denied,
        quotes::QuoteStatus::Offered { keyset_id, ttl } => web_quotes::StatusReply::Offered {
            keyset_id,
            expiration_date: ttl,
        },
        quotes::QuoteStatus::Rejected { tstamp } => web_quotes::StatusReply::Rejected { tstamp },
        quotes::QuoteStatus::Accepted { keyset_id } => {
            web_quotes::StatusReply::Accepted { keyset_id }
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

    let quote = ctrl.lookup(id).await?;
    Ok(Json(convert_to_enquire_reply(quote)))
}

#[utoipa::path(
    post,
    path = "/v1/credit/quote/{id}",
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

    match req {
        web_quotes::ResolveOffer::Reject => ctrl.reject(id, chrono::Utc::now()).await?,
        web_quotes::ResolveOffer::Accept => ctrl.accept(id).await?,
    }
    Ok(())
}
