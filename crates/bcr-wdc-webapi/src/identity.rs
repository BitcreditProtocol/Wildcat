// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use bcr_common::wire::identity as wire_identity;
use bcr_ebill_core::{self as data, identity, NodeId};
use bcr_wdc_utils::convert;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
// ----- local imports

// ----- end imports

#[derive(Debug, Error)]
pub enum Error {
    #[error("Invalid bitcoin key")]
    InvalidBitcoinKey,
    #[error("Invalid URL")]
    InvalidUrl,
    #[error("Invalid date")]
    InvalidDate(chrono::ParseError),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Identity {
    pub node_id: NodeId,
    pub name: String,
    pub email: Option<String>,
    pub bitcoin_public_key: bitcoin::PublicKey,
    pub npub: String,
    pub postal_address: wire_identity::OptionalPostalAddress,
    pub date_of_birth: Option<NaiveDate>,
    pub country_of_birth: Option<String>,
    pub city_of_birth: Option<String>,
    pub identification_number: Option<String>,
    pub profile_picture_file: Option<File>,
    pub identity_document_file: Option<File>,
    pub nostr_relays: Vec<url::Url>,
}

impl TryFrom<identity::Identity> for Identity {
    type Error = Error;
    fn try_from(identity: identity::Identity) -> Result<Self, Self::Error> {
        let nostr_relays: Vec<url::Url> = identity
            .nostr_relays
            .iter()
            .map(|r| url::Url::parse(r).map_err(|_| Self::Error::InvalidUrl))
            .collect::<Result<_, _>>()?;

        let date_of_birth: Option<NaiveDate> = identity
            .date_of_birth
            .as_deref()
            .map(NaiveDate::from_str)
            .transpose()
            .map_err(Self::Error::InvalidDate)?;
        Ok(Self {
            node_id: identity.node_id.clone(),
            name: identity.name,
            email: identity.email,
            bitcoin_public_key: identity.node_id.pub_key().into(),
            npub: identity.node_id.npub().to_string(),
            postal_address: convert::optionalpostaladdress_ebill2wire(identity.postal_address),
            date_of_birth,
            country_of_birth: identity.country_of_birth,
            city_of_birth: identity.city_of_birth,
            identification_number: identity.identification_number,
            profile_picture_file: identity.profile_picture_file.map(|f| f.into()),
            identity_document_file: identity.identity_document_file.map(|f| f.into()),
            nostr_relays,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NewIdentityPayload {
    pub t: u64,
    pub name: String,
    pub email: Option<String>,
    pub postal_address: wire_identity::OptionalPostalAddress,
    pub date_of_birth: Option<String>,
    pub country_of_birth: Option<String>,
    pub city_of_birth: Option<String>,
    pub identification_number: Option<String>,
    pub profile_picture_file_upload_id: Option<String>,
    pub identity_document_file_upload_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct File {
    pub name: String,
    pub hash: String,
    pub nostr_hash: String,
}

impl From<data::File> for File {
    fn from(val: data::File) -> Self {
        File {
            name: val.name,
            hash: val.hash,
            nostr_hash: val.nostr_hash,
        }
    }
}
