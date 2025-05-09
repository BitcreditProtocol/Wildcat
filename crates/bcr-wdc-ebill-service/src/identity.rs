use std::str::FromStr;

// ----- standard library imports
// ----- extra library imports
use bcr_ebill_api::{
    data::{
        identity::{Identity, IdentityType},
        File, OptionalPostalAddress, PostalAddress,
    },
    util::BcrKeys,
};
use serde::{Deserialize, Serialize};
// ----- local imports
use crate::error::{Error, Result};
// ----- end imports

#[repr(u8)]
#[derive(
    Debug, Copy, Clone, serde_repr::Serialize_repr, serde_repr::Deserialize_repr, PartialEq, Eq,
)]
pub enum IdentityTypeWeb {
    Ident = 0,
    Anon = 1,
}

impl TryFrom<u64> for IdentityTypeWeb {
    type Error = bcr_ebill_api::service::Error;

    fn try_from(value: u64) -> std::result::Result<Self, Self::Error> {
        Ok(IdentityType::try_from(value)
            .map_err(Self::Error::Validation)?
            .into())
    }
}

impl From<IdentityType> for IdentityTypeWeb {
    fn from(val: IdentityType) -> Self {
        match val {
            IdentityType::Ident => IdentityTypeWeb::Ident,
            IdentityType::Anon => IdentityTypeWeb::Anon,
        }
    }
}

impl From<IdentityTypeWeb> for IdentityType {
    fn from(value: IdentityTypeWeb) -> Self {
        match value {
            IdentityTypeWeb::Ident => IdentityType::Ident,
            IdentityTypeWeb::Anon => IdentityType::Anon,
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SeedPhrase {
    pub seed_phrase: bip39::Mnemonic,
}

#[derive(Debug, Serialize)]
pub struct IdentityWeb {
    pub node_id: String,
    pub name: String,
    pub email: Option<String>,
    pub bitcoin_public_key: bitcoin::PublicKey,
    pub npub: String,
    pub postal_address: OptionalPostalAddressWeb,
    pub date_of_birth: Option<String>,
    pub country_of_birth: Option<String>,
    pub city_of_birth: Option<String>,
    pub identification_number: Option<String>,
    pub profile_picture_file: Option<FileWeb>,
    pub identity_document_file: Option<FileWeb>,
    pub nostr_relays: Vec<url::Url>,
}

impl TryFrom<(Identity, BcrKeys)> for IdentityWeb {
    type Error = Error;
    fn try_from((identity, keys): (Identity, BcrKeys)) -> Result<Self> {
        let nostr_relays: Vec<url::Url> = identity
            .nostr_relays
            .iter()
            .map(|r| url::Url::parse(r).map_err(|_| Error::InvalidUrl))
            .collect::<Result<_>>()?;

        Ok(Self {
            node_id: identity.node_id.clone(),
            name: identity.name,
            email: identity.email,
            bitcoin_public_key: bitcoin::PublicKey::from_str(&identity.node_id)
                .map_err(|_| Error::InvalidBitcoinKey)?,
            npub: keys.get_nostr_npub(),
            postal_address: identity.postal_address.into(),
            date_of_birth: identity.date_of_birth,
            country_of_birth: identity.country_of_birth,
            city_of_birth: identity.city_of_birth,
            identification_number: identity.identification_number,
            profile_picture_file: identity.profile_picture_file.map(|f| f.into()),
            identity_document_file: identity.identity_document_file.map(|f| f.into()),
            nostr_relays,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct NewIdentityPayload {
    pub t: u64,
    pub name: String,
    pub email: Option<String>,
    pub postal_address: OptionalPostalAddressWeb,
    pub date_of_birth: Option<String>,
    pub country_of_birth: Option<String>,
    pub city_of_birth: Option<String>,
    pub identification_number: Option<String>,
    pub profile_picture_file_upload_id: Option<String>,
    pub identity_document_file_upload_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostalAddressWeb {
    pub country: String,
    pub city: String,
    pub zip: Option<String>,
    pub address: String,
}

impl From<PostalAddress> for PostalAddressWeb {
    fn from(val: PostalAddress) -> Self {
        PostalAddressWeb {
            country: val.country,
            city: val.city,
            zip: val.zip,
            address: val.address,
        }
    }
}

impl From<PostalAddressWeb> for PostalAddress {
    fn from(value: PostalAddressWeb) -> Self {
        Self {
            country: value.country,
            city: value.city,
            zip: value.zip,
            address: value.address,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionalPostalAddressWeb {
    pub country: Option<String>,
    pub city: Option<String>,
    pub zip: Option<String>,
    pub address: Option<String>,
}

impl OptionalPostalAddressWeb {
    pub fn is_none(&self) -> bool {
        self.country.is_none()
            && self.city.is_none()
            && self.zip.is_none()
            && self.address.is_none()
    }
}

impl From<OptionalPostalAddress> for OptionalPostalAddressWeb {
    fn from(value: OptionalPostalAddress) -> Self {
        Self {
            country: value.country,
            city: value.city,
            zip: value.zip,
            address: value.address,
        }
    }
}

impl From<OptionalPostalAddressWeb> for OptionalPostalAddress {
    fn from(value: OptionalPostalAddressWeb) -> Self {
        Self {
            country: value.country,
            city: value.city,
            zip: value.zip,
            address: value.address,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileWeb {
    pub name: String,
    pub hash: String,
}

impl From<File> for FileWeb {
    fn from(val: File) -> Self {
        FileWeb {
            name: val.name,
            hash: val.hash,
        }
    }
}
