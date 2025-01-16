// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
use rust_decimal::Decimal;
// ----- local modules
// ----- local imports
use crate::credit::{mint, Controller, Result};

///--------------------------- Enquire mint quote
#[derive(serde::Deserialize)]
pub struct QuoteRequest {
    bill: String,
    node: String,
}

#[derive(serde::Serialize)]
pub struct QuoteRequestReply {
    id: uuid::Uuid,
}

pub async fn enquire_quote(
    State(ctrl): State<Controller>,
    Json(req): Json<QuoteRequest>,
) -> Result<Json<QuoteRequestReply>> {
    log::debug!(
        "Received mint quote request for bill: {}, from node : {}",
        req.bill,
        req.node
    );

    let QuoteRequest { bill, node } = req;
    let id = ctrl.quote_service.enquire(bill, node, chrono::Utc::now())?;

    Ok(Json(QuoteRequestReply { id }))
}

/// --------------------------- Look up quote
#[derive(serde::Serialize)]
pub enum LookUpQuoteReply {
    Pending,
    Declined,
    Accepted {
        discount: Decimal,
        ttl: chrono::DateTime<chrono::Utc>,
    },
}

impl std::convert::From<mint::Quote> for LookUpQuoteReply {
    fn from(quote: mint::Quote) -> Self {
        match quote {
            mint::Quote::Pending(_) => LookUpQuoteReply::Pending,
            mint::Quote::Declined(_) => LookUpQuoteReply::Declined,
            mint::Quote::Accepted(_, details) => LookUpQuoteReply::Accepted {
                discount: details.discounted,
                ttl: details.ttl,
            },
        }
    }
}

pub async fn lookup_quote(
    State(ctrl): State<Controller>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<LookUpQuoteReply>> {
    log::debug!("Received mint quote lookup request for id: {}", id);

    let service = ctrl.quote_service.clone();
    let quote = service.lookup(id)?;
    let response = LookUpQuoteReply::from(quote);
    Ok(Json(response))
}
