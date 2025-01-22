// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, State};
use axum::routing::Router;
use axum::routing::{get, post};
use cdk::nuts::nut00 as cdk00;
// ----- local modules
// ----- local imports
use super::quotes;
use super::{Controller, Result};

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

pub async fn enquire_quote(
    State(ctrl): State<Controller>,
    Json(req): Json<QuoteRequest>,
) -> Result<Json<QuoteRequestReply>> {
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
        match quote.status() {
            quotes::QuoteStatus::Pending { .. } => LookUpQuoteReply::Pending,
            quotes::QuoteStatus::Declined => LookUpQuoteReply::Declined,
            quotes::QuoteStatus::Accepted { signatures, ttl } => LookUpQuoteReply::Accepted {
                signatures: signatures.clone(),
                expiration_date: *ttl,
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
    Ok(Json(quote.into()))
}

pub fn routes(ctrl: Controller) -> Router {
    let v1_credit = Router::new()
        .route("/mint/quote", post(enquire_quote))
        .route("/mint/quote/:id", get(lookup_quote));

    Router::new().nest("/credit/v1", v1_credit).with_state(ctrl)
}
