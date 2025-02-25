// ----- standard library imports
// ----- extra library imports
use bcr_ebill_core::contact::IdentityPublicData;
use cashu::nuts::nut00::{BlindSignature, BlindedMessage};
use rust_decimal::Decimal;
// ----- local imports

///--------------------------- Enquire mint quote
#[derive(
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshSerialize,
    borsh::BorshDeserialize,
    utoipa::ToSchema,
)]
pub struct BillInfo {
    pub id: String,
    pub drawee: IdentityPublicData,
    pub drawer: IdentityPublicData,
    pub payee: IdentityPublicData,
    pub holder: IdentityPublicData,
    pub sum: u64,
    pub maturity_date: String,
}

///--------------------------- Enquire mint quote
#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct EnquireRequest {
    pub content: BillInfo,
    #[schema(value_type = String)]
    pub signature: bitcoin::secp256k1::schnorr::Signature,

    pub outputs: Vec<BlindedMessage>, // left out of the signature as BlindedMessage does not implement borsh
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
        expiration_date: chrono::DateTime<chrono::Utc>,
    },
    Accepted {
        signatures: Vec<BlindSignature>,
    },
    Rejected {
        tstamp: chrono::DateTime<chrono::Utc>,
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
        bill: BillInfo,
        submitted: chrono::DateTime<chrono::Utc>,
        suggested_expiration: chrono::DateTime<chrono::Utc>,
    },
    Offered {
        id: uuid::Uuid,
        bill: BillInfo,
        ttl: chrono::DateTime<chrono::Utc>,
        signatures: Vec<BlindSignature>,
    },
    Denied {
        id: uuid::Uuid,
        bill: BillInfo,
    },
    Accepted {
        id: uuid::Uuid,
        bill: BillInfo,
        signatures: Vec<BlindSignature>,
    },
    Rejected {
        id: uuid::Uuid,
        bill: BillInfo,
        tstamp: chrono::DateTime<chrono::Utc>,
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
