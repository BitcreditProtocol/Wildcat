// ----- standard library imports
// ----- extra library imports
use cdk::nuts::nut00::{BlindSignature, BlindedMessage};
use rust_decimal::Decimal;
// ----- local imports

type TStamp = chrono::DateTime<chrono::Utc>;

///--------------------------- Enquire mint quote
#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct EnquireRequest {
    pub bill: String,
    pub node: String,
    pub outputs: Vec<BlindedMessage>,
}

#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct EnquireReply {
    pub id: uuid::Uuid,
}

/// --------------------------- Look up quote
#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase", tag = "status")]
pub enum StatusReply {
    Pending,
    Denied,
    Offered {
        signatures: Vec<BlindSignature>,
        expiration_date: TStamp,
    },
    Accepted {
        signatures: Vec<BlindSignature>,
    },
    Rejected {
        tstamp: TStamp,
    },
}

/// --------------------------- List quotes
#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct ListReply {
    pub quotes: Vec<uuid::Uuid>,
}

/// --------------------------- Quote info request
#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase", tag = "status")]
pub enum InfoReply {
    Pending {
        id: uuid::Uuid,
        bill: String,
        endorser: String,
        submitted: chrono::DateTime<chrono::Utc>,
        suggested_expiration: chrono::DateTime<chrono::Utc>,
    },
    Offered {
        id: uuid::Uuid,
        bill: String,
        endorser: String,
        ttl: chrono::DateTime<chrono::Utc>,
        signatures: Vec<BlindSignature>,
    },
    Denied {
        id: uuid::Uuid,
        bill: String,
        endorser: String,
    },
    Accepted {
        id: uuid::Uuid,
        bill: String,
        endorser: String,
        signatures: Vec<BlindSignature>,
    },
    Rejected {
        id: uuid::Uuid,
        bill: String,
        endorser: String,
        tstamp: TStamp,
    },
}

/// --------------------------- Resolve quote request
#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase", tag = "action")]
pub enum ResolveRequest {
    Deny,
    Offer {
        discount: Decimal,
        ttl: Option<chrono::DateTime<chrono::Utc>>,
    },
}

/// --------------------------- Resolve quote
#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase", tag = "action")]
pub enum ResolveOffer {
    Reject,
    Accept,
}
