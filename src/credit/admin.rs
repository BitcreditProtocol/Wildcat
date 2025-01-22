// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, Query, State};
use axum::routing::{get, post, Router};
use cdk::nuts::nut00 as cdk00;
use rust_decimal::Decimal;
use uuid::Uuid;
// ----- local modules
// ----- local modules
use super::{quotes, utils, Controller, Result, TStamp};

/// --------------------------- List quotes
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ListQuotesReply {
    pub quotes: Vec<uuid::Uuid>,
}

pub async fn list_pending_quotes(
    State(ctrl): State<Controller>,
    since: Option<Query<TStamp>>,
) -> Result<Json<ListQuotesReply>> {
    log::debug!("Received request to list pending quotes");

    let quotes = ctrl.list_pendings(since.map(|q| q.0))?;
    Ok(Json(ListQuotesReply { quotes }))
}

pub async fn list_accepted_quotes(State(ctrl): State<Controller>) -> Result<Json<ListQuotesReply>> {
    log::debug!("Received request to list accepted quotes");

    let quotes = ctrl.list_accepteds()?;
    Ok(Json(ListQuotesReply { quotes }))
}

/// --------------------------- Look up request
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase", tag = "status")]
pub enum LookUpQuoteReply {
    Pending {
        id: Uuid,
        bill: String,
        endorser: String,
        submitted: chrono::DateTime<chrono::Utc>,
        suggested_expiration: chrono::DateTime<chrono::Utc>,
    },
    Accepted {
        id: Uuid,
        bill: String,
        endorser: String,
        ttl: chrono::DateTime<chrono::Utc>,
        signatures: Vec<cdk00::BlindSignature>,
    },
    Declined {
        id: Uuid,
        bill: String,
        endorser: String,
    },
}

impl std::convert::From<quotes::Quote> for LookUpQuoteReply {
    fn from(quote: quotes::Quote) -> Self {
        match quote.status() {
            quotes::QuoteStatus::Pending { .. } => LookUpQuoteReply::Pending {
                id: quote.id,
                bill: quote.bill,
                endorser: quote.endorser,
                submitted: quote.submitted,
                suggested_expiration: utils::calculate_default_expiration_date_for_quote(
                    chrono::Utc::now(),
                ),
            },
            quotes::QuoteStatus::Accepted { signatures, ttl } => LookUpQuoteReply::Accepted {
                id: quote.id,
                bill: quote.bill.clone(),
                endorser: quote.endorser.clone(),
                ttl: *ttl,
                signatures: signatures.clone(),
            },
            quotes::QuoteStatus::Declined => LookUpQuoteReply::Declined {
                id: quote.id,
                bill: quote.bill,
                endorser: quote.endorser,
            },
        }
    }
}

pub async fn lookup_quote(
    State(ctrl): State<Controller>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<LookUpQuoteReply>> {
    log::debug!("Received mint quote lookup request for id: {}", id);

    let quote = ctrl.lookup(id)?;
    let response = LookUpQuoteReply::from(quote);
    Ok(Json(response))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase", tag = "action")]
pub enum ResolveQuoteRequest {
    Decline,
    Accept {
        discount: Decimal,
        ttl: Option<chrono::DateTime<chrono::Utc>>,
    },
}

pub async fn resolve_quote(
    State(ctrl): State<Controller>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<ResolveQuoteRequest>,
) -> Result<()> {
    log::debug!("Received mint quote resolve request for id: {}", id);

    match req {
        ResolveQuoteRequest::Decline => ctrl.decline(id)?,
        ResolveQuoteRequest::Accept { discount, ttl } => {
            ctrl.accept(id, discount, chrono::Utc::now(), ttl)?
        }
    }
    Ok(())
}

pub fn routes(ctrl: Controller) -> Router {
    let admin = Router::new()
        .route("/quote/pending", get(list_pending_quotes))
        .route("/quote/accepted", get(list_accepted_quotes))
        .route("/quote/:id", get(lookup_quote))
        .route("/quote/:id", post(resolve_quote));
    Router::new()
        .nest("/admin/credit/v1", admin)
        .with_state(ctrl)
}
