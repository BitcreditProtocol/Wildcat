// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
use cdk::nuts::nut00 as cdk00;
// ----- local modules
// ----- local imports
use crate::credit::error::Result;
use crate::credit::quotes;

///--------------------------- Enquire mint quote
#[derive(serde::Deserialize)]
pub struct QuoteRequest {
    bill: String,
    node: String,
    outputs: Vec<cdk00::BlindedMessage>,
}

#[derive(serde::Serialize)]
pub struct QuoteRequestReply {
    id: uuid::Uuid,
}

pub async fn enquire_quote<KG, QR>(
    State(ctrl): State<quotes::Service<KG, QR>>,
    Json(req): Json<QuoteRequest>,
) -> Result<Json<QuoteRequestReply>>
where
    KG: quotes::KeyFactory,
    QR: quotes::Repository,
{
    log::debug!(
        "Received mint quote request for bill: {}, from node : {}",
        req.bill,
        req.node
    );

    let id = ctrl.enquire(req.bill, req.node, chrono::Utc::now(), req.outputs)?;
    Ok(Json(QuoteRequestReply { id }))
}

/// --------------------------- Look up quote
#[derive(serde::Serialize)]
#[serde(rename_all = "lowercase", tag = "status")]
pub enum LookUpQuoteReply {
    Pending,
    Declined,
    Accepted {
        signatures: Vec<cdk00::BlindSignature>,
        expiration_date: chrono::DateTime<chrono::Utc>,
    },
}

impl std::convert::From<quotes::Quote> for LookUpQuoteReply {
    fn from(quote: quotes::Quote) -> Self {
        match quote.status {
            quotes::QuoteStatus::Pending { .. } => LookUpQuoteReply::Pending,
            quotes::QuoteStatus::Declined => LookUpQuoteReply::Declined,
            quotes::QuoteStatus::Accepted { signatures, ttl } => LookUpQuoteReply::Accepted {
                signatures,
                expiration_date: ttl,
            },
        }
    }
}

pub async fn lookup_quote<KG, QR>(
    State(ctrl): State<quotes::Service<KG, QR>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<LookUpQuoteReply>>
where
    KG: quotes::KeyFactory,
    QR: quotes::Repository,
{
    log::debug!("Received mint quote lookup request for id: {}", id);

    let quote = ctrl.lookup(id)?;
    Ok(Json(quote.into()))
}
