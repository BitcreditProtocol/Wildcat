// ----- standard library imports
// ----- extra library imports
use cdk::nuts::nut00 as cdk00;
use rust_decimal::Decimal;
// ----- local imports

type TStamp = chrono::DateTime<chrono::Utc>;

///--------------------------- Enquire mint quote
#[derive(serde::Deserialize)]
pub struct EnquireRequest {
    pub bill: String,
    pub node: String,
    pub outputs: Vec<cdk00::BlindedMessage>,
}

#[derive(serde::Serialize)]
pub struct EnquireReply {
    pub id: uuid::Uuid,
}

/// --------------------------- Look up quote
#[derive(serde::Serialize)]
#[serde(rename_all = "lowercase", tag = "status")]
pub enum StatusReply {
    Pending,
    Declined,
    Accepted {
        signatures: Vec<cdk00::BlindSignature>,
        expiration_date: TStamp,
    },
}

/// --------------------------- List quotes
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ListReply {
    pub quotes: Vec<uuid::Uuid>,
}

/// --------------------------- Quote info request
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase", tag = "status")]
pub enum InfoReply {
    Pending {
        id: uuid::Uuid,
        bill: String,
        endorser: String,
        submitted: chrono::DateTime<chrono::Utc>,
        suggested_expiration: chrono::DateTime<chrono::Utc>,
    },
    Accepted {
        id: uuid::Uuid,
        bill: String,
        endorser: String,
        ttl: chrono::DateTime<chrono::Utc>,
        signatures: Vec<cdk00::BlindSignature>,
    },
    Declined {
        id: uuid::Uuid,
        bill: String,
        endorser: String,
    },
}

/// --------------------------- Resolve quote request
#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase", tag = "action")]
pub enum ResolveRequest {
    Decline,
    Accept {
        discount: Decimal,
        ttl: Option<chrono::DateTime<chrono::Utc>>,
    },
}
