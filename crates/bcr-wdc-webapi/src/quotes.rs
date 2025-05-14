// ----- standard library imports
// ----- extra library imports
use borsh::{BorshDeserialize, BorshSerialize};
use cashu::{nut01 as cdk01, nut02 as cdk02};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::bill::{BillIdentParticipant, BillParticipant};
// ----- local imports

// ----- end imports

///--------------------------- Enquire mint quote
#[derive(Serialize, Deserialize, BorshSerialize, BorshDeserialize, ToSchema)]
pub struct BillInfo {
    pub id: String,
    pub drawee: BillIdentParticipant,
    pub drawer: BillIdentParticipant,
    pub payee: BillParticipant,
    pub endorsees: Vec<BillParticipant>,
    pub sum: u64, // in satoshis, converted to bitcoin::Amount in the service
    pub maturity_date: String,
}

///--------------------------- Enquire mint quote
#[derive(Serialize, Deserialize, ToSchema)]
pub struct EnquireRequest {
    pub content: BillInfo,
    #[schema(value_type = String)]
    pub signature: bitcoin::secp256k1::schnorr::Signature,

    pub public_key: cdk01::PublicKey, // left out of the signature as BlindedMessage does not implement borsh
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct EnquireReply {
    pub id: uuid::Uuid,
}

/// --------------------------- Look up quote
#[derive(Serialize, Deserialize, ToSchema, strum::EnumDiscriminants)]
#[strum_discriminants(derive(Serialize, Deserialize, ToSchema))]
#[serde(tag = "status")]
pub enum StatusReply {
    Pending,
    Denied,
    Offered {
        keyset_id: cdk02::Id,
        expiration_date: chrono::DateTime<chrono::Utc>,
        #[schema(value_type = u64)]
        discounted: bitcoin::Amount,
    },
    Accepted {
        keyset_id: cdk02::Id,
    },
    Rejected {
        tstamp: chrono::DateTime<chrono::Utc>,
    },
}

/// --------------------------- List quotes
#[derive(Default, Serialize, Deserialize, IntoParams, Debug)]
pub struct ListParam {
    pub bill_maturity_date_from: Option<chrono::NaiveDate>,
    pub bill_maturity_date_to: Option<chrono::NaiveDate>,
    pub status: Option<StatusReplyDiscriminants>,
    pub bill_drawee_id: Option<String>,
    pub bill_drawer_id: Option<String>,
    pub bill_payer_id: Option<String>,
    pub bill_holder_id: Option<String>,
    pub sort: Option<ListSort>,
}

#[derive(Serialize, Deserialize, ToSchema, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ListSort {
    BillMaturityDateDesc,
    BillMaturityDateAsc,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct ListReply {
    pub quotes: Vec<uuid::Uuid>,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct LightInfo {
    pub id: uuid::Uuid,
    pub status: StatusReplyDiscriminants,
    #[schema(value_type = u64)]
    pub sum: bitcoin::Amount,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct ListReplyLight {
    pub quotes: Vec<LightInfo>,
}

/// --------------------------- Quote info request
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "PascalCase", tag = "status")]
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
        keyset_id: cdk02::Id,
    },
    Denied {
        id: uuid::Uuid,
        bill: BillInfo,
    },
    Accepted {
        id: uuid::Uuid,
        bill: BillInfo,
        keyset_id: cdk02::Id,
    },
    Rejected {
        id: uuid::Uuid,
        bill: BillInfo,
        tstamp: chrono::DateTime<chrono::Utc>,
    },
}

/// --------------------------- Update quote status request
#[derive(Serialize, Deserialize, ToSchema, Debug)]
#[serde(rename_all = "PascalCase", tag = "action")]
pub enum UpdateQuoteRequest {
    Deny,
    Offer {
        #[schema(value_type = u64)]
        discounted: bitcoin::Amount,
        ttl: Option<chrono::DateTime<chrono::Utc>>,
    },
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "PascalCase", tag = "status")]
pub enum UpdateQuoteResponse {
    Denied,
    Offered {
        #[schema(value_type = u64)]
        discounted: bitcoin::Amount,
        ttl: chrono::DateTime<chrono::Utc>,
    },
}

/// --------------------------- Resolve quote
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "PascalCase", tag = "action")]
pub enum ResolveOffer {
    Reject,
    Accept,
}
