use bcr_ebill_core::contact;
// ----- standard library imports
// ----- extra library imports
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
// ----- local modules
use crate::identity::{File, PostalAddress};
// ----- end imports

#[repr(u8)]
#[derive(
    Debug,
    Copy,
    Clone,
    serde_repr::Serialize_repr,
    serde_repr::Deserialize_repr,
    PartialEq,
    Eq,
    BorshSerialize,
    BorshDeserialize,
    ToSchema,
)]
#[borsh(use_discriminant = true)]
pub enum ContactType {
    Person = 0,
    Company = 1,
    Anon = 2,
}

impl TryFrom<u64> for ContactType {
    type Error = bcr_ebill_core::ValidationError;

    fn try_from(value: u64) -> std::result::Result<Self, Self::Error> {
        Ok(contact::ContactType::try_from(value)?.into())
    }
}

impl From<contact::ContactType> for ContactType {
    fn from(val: contact::ContactType) -> Self {
        match val {
            contact::ContactType::Person => ContactType::Person,
            contact::ContactType::Company => ContactType::Company,
            contact::ContactType::Anon => ContactType::Anon,
        }
    }
}

impl From<ContactType> for contact::ContactType {
    fn from(value: ContactType) -> Self {
        match value {
            ContactType::Person => contact::ContactType::Person,
            ContactType::Company => contact::ContactType::Company,
            ContactType::Anon => contact::ContactType::Anon,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NewContactPayload {
    pub t: u64,
    pub node_id: String,
    pub name: String,
    pub email: Option<String>,
    pub postal_address: Option<PostalAddress>,
    pub date_of_birth_or_registration: Option<String>,
    pub country_of_birth_or_registration: Option<String>,
    pub city_of_birth_or_registration: Option<String>,
    pub identification_number: Option<String>,
    pub avatar_file_upload_id: Option<String>,
    pub proof_document_file_upload_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Contact {
    pub t: ContactType,
    pub node_id: String,
    pub name: String,
    pub email: Option<String>,
    pub postal_address: Option<PostalAddress>,
    pub date_of_birth_or_registration: Option<String>,
    pub country_of_birth_or_registration: Option<String>,
    pub city_of_birth_or_registration: Option<String>,
    pub identification_number: Option<String>,
    pub avatar_file: Option<File>,
    pub proof_document_file: Option<File>,
    pub nostr_relays: Vec<String>,
}

impl From<contact::Contact> for Contact {
    fn from(val: contact::Contact) -> Self {
        Contact {
            t: val.t.into(),
            node_id: val.node_id,
            name: val.name,
            email: val.email,
            postal_address: val.postal_address.map(|pa| pa.into()),
            date_of_birth_or_registration: val.date_of_birth_or_registration,
            country_of_birth_or_registration: val.country_of_birth_or_registration,
            city_of_birth_or_registration: val.city_of_birth_or_registration,
            identification_number: val.identification_number,
            avatar_file: val.avatar_file.map(|f| f.into()),
            proof_document_file: val.proof_document_file.map(|f| f.into()),
            nostr_relays: val.nostr_relays,
        }
    }
}
