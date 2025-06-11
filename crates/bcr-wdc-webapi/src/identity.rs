use std::str::FromStr;
use thiserror::Error;

// ----- standard library imports
// ----- extra library imports
use bcr_ebill_core::{self as data, identity, util::BcrKeys};
use borsh::{BorshDeserialize, BorshSerialize};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
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

#[repr(u8)]
#[derive(
    Debug,
    Copy,
    Clone,
    serde_repr::Serialize_repr,
    serde_repr::Deserialize_repr,
    PartialEq,
    Eq,
    ToSchema,
)]
pub enum IdentityType {
    Ident = 0,
    Anon = 1,
}

impl TryFrom<u64> for IdentityType {
    type Error = bcr_ebill_core::ValidationError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Ok(identity::IdentityType::try_from(value)?.into())
    }
}

impl From<identity::IdentityType> for IdentityType {
    fn from(val: identity::IdentityType) -> Self {
        match val {
            identity::IdentityType::Ident => IdentityType::Ident,
            identity::IdentityType::Anon => IdentityType::Anon,
        }
    }
}

impl From<IdentityType> for identity::IdentityType {
    fn from(value: IdentityType) -> Self {
        match value {
            IdentityType::Ident => identity::IdentityType::Ident,
            IdentityType::Anon => identity::IdentityType::Anon,
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SeedPhrase {
    pub seed_phrase: bip39::Mnemonic,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Identity {
    pub node_id: String,
    pub name: String,
    pub email: Option<String>,
    pub bitcoin_public_key: bitcoin::PublicKey,
    pub npub: String,
    pub postal_address: OptionalPostalAddress,
    pub date_of_birth: Option<NaiveDate>,
    pub country_of_birth: Option<String>,
    pub city_of_birth: Option<String>,
    pub identification_number: Option<String>,
    pub profile_picture_file: Option<File>,
    pub identity_document_file: Option<File>,
    pub nostr_relays: Vec<url::Url>,
}

impl TryFrom<(identity::Identity, BcrKeys)> for Identity {
    type Error = Error;
    fn try_from((identity, keys): (identity::Identity, BcrKeys)) -> Result<Self, Self::Error> {
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
            bitcoin_public_key: bitcoin::PublicKey::from_str(&identity.node_id)
                .map_err(|_| Error::InvalidBitcoinKey)?,
            npub: keys.get_nostr_npub(),
            postal_address: identity.postal_address.into(),
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
    pub postal_address: OptionalPostalAddress,
    pub date_of_birth: Option<String>,
    pub country_of_birth: Option<String>,
    pub city_of_birth: Option<String>,
    pub identification_number: Option<String>,
    pub profile_picture_file_upload_id: Option<String>,
    pub identity_document_file_upload_id: Option<String>,
}

#[derive(
    Debug, Default, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize, ToSchema,
)]
pub struct PostalAddress {
    pub country: String,
    pub city: String,
    pub zip: Option<String>,
    pub address: String,
}

impl From<data::PostalAddress> for PostalAddress {
    fn from(val: data::PostalAddress) -> Self {
        PostalAddress {
            country: val.country,
            city: val.city,
            zip: val.zip,
            address: val.address,
        }
    }
}

impl From<PostalAddress> for data::PostalAddress {
    fn from(value: PostalAddress) -> Self {
        Self {
            country: value.country,
            city: value.city,
            zip: value.zip,
            address: value.address,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize, ToSchema)]
pub struct OptionalPostalAddress {
    pub country: Option<String>,
    pub city: Option<String>,
    pub zip: Option<String>,
    pub address: Option<String>,
}

impl OptionalPostalAddress {
    pub fn is_none(&self) -> bool {
        self.country.is_none()
            && self.city.is_none()
            && self.zip.is_none()
            && self.address.is_none()
    }
}

impl From<data::OptionalPostalAddress> for OptionalPostalAddress {
    fn from(value: data::OptionalPostalAddress) -> Self {
        Self {
            country: value.country,
            city: value.city,
            zip: value.zip,
            address: value.address,
        }
    }
}

impl From<OptionalPostalAddress> for data::OptionalPostalAddress {
    fn from(value: OptionalPostalAddress) -> Self {
        Self {
            country: value.country,
            city: value.city,
            zip: value.zip,
            address: value.address,
        }
    }
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
