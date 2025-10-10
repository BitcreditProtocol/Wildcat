// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use bcr_common::{
    core,
    wire::{bill as wire_bill, contact as wire_contact, identity as wire_identity},
};
use bcr_ebill_core::{self as ebill_core, contact as ebill_contact, identity as ebill_identity};
use thiserror::Error;
// ----- local imports

// ----- end imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("Chrono parse {0}")]
    ChronoParse(#[from] chrono::ParseError),
    #[error("Url parse {0}")]
    UrlParse(#[from] url::ParseError),
}

pub fn identitytype_wire2ebill(input: wire_identity::IdentityType) -> ebill_identity::IdentityType {
    match input {
        wire_identity::IdentityType::Ident => ebill_identity::IdentityType::Ident,
        wire_identity::IdentityType::Anon => ebill_identity::IdentityType::Anon,
    }
}

pub fn postaladdress_ebill2wire(input: ebill_core::PostalAddress) -> wire_identity::PostalAddress {
    wire_identity::PostalAddress {
        country: input.country,
        city: input.city,
        zip: input.zip,
        address: input.address,
    }
}

pub fn postaladdress_wire2ebill(input: wire_identity::PostalAddress) -> ebill_core::PostalAddress {
    ebill_core::PostalAddress {
        country: input.country,
        city: input.city,
        zip: input.zip,
        address: input.address,
    }
}

pub fn optionalpostaladdress_ebill2wire(
    input: ebill_core::OptionalPostalAddress,
) -> wire_identity::OptionalPostalAddress {
    wire_identity::OptionalPostalAddress {
        country: input.country,
        city: input.city,
        zip: input.zip,
        address: input.address,
    }
}

pub fn optionalpostaladdress_wire2ebill(
    input: wire_identity::OptionalPostalAddress,
) -> ebill_core::OptionalPostalAddress {
    ebill_core::OptionalPostalAddress {
        country: input.country,
        city: input.city,
        zip: input.zip,
        address: input.address,
    }
}

pub fn file_ebill2wire(input: ebill_core::File) -> wire_identity::File {
    wire_identity::File {
        name: input.name,
        hash: input.hash,
        nostr_hash: input.nostr_hash,
    }
}

pub fn contacttype_ebill2wire(input: ebill_contact::ContactType) -> wire_contact::ContactType {
    match input {
        ebill_contact::ContactType::Person => wire_contact::ContactType::Person,
        ebill_contact::ContactType::Company => wire_contact::ContactType::Company,
        ebill_contact::ContactType::Anon => wire_contact::ContactType::Anon,
    }
}

pub fn contacttype_wire2ebill(input: wire_contact::ContactType) -> ebill_contact::ContactType {
    match input {
        wire_contact::ContactType::Person => ebill_contact::ContactType::Person,
        wire_contact::ContactType::Company => ebill_contact::ContactType::Company,
        wire_contact::ContactType::Anon => ebill_contact::ContactType::Anon,
    }
}

pub fn nodeid_ebill2wire(input: ebill_core::NodeId) -> core::NodeId {
    core::NodeId::new(input.pub_key(), input.network())
}

pub fn identity_ebill2wire(input: ebill_identity::Identity) -> Result<wire_identity::Identity> {
    let date_of_birth = input
        .date_of_birth
        .as_deref()
        .map(chrono::NaiveDate::from_str)
        .transpose()?;
    let nostr_relays = input
        .nostr_relays
        .iter()
        .map(String::as_str)
        .map(reqwest::Url::parse)
        .collect::<std::result::Result<_, _>>()?;
    let output = wire_identity::Identity {
        node_id: nodeid_ebill2wire(input.node_id.clone()),
        name: input.name,
        email: input.email,
        bitcoin_public_key: input.node_id.pub_key().into(),
        npub: input.node_id.npub().to_string(),
        postal_address: optionalpostaladdress_ebill2wire(input.postal_address),
        date_of_birth,
        country_of_birth: input.country_of_birth,
        city_of_birth: input.city_of_birth,
        identification_number: input.identification_number,
        profile_picture_file: input.profile_picture_file.map(file_ebill2wire),
        identity_document_file: input.identity_document_file.map(file_ebill2wire),
        nostr_relays,
    };
    Ok(output)
}

pub fn lightbillidentparticipantwithaddress_ebill2wire(
    input: ebill_contact::LightBillIdentParticipantWithAddress,
) -> wire_bill::LightBillIdentParticipantWithAddress {
    wire_bill::LightBillIdentParticipantWithAddress {
        t: contacttype_ebill2wire(input.t),
        name: input.name,
        node_id: nodeid_ebill2wire(input.node_id),
        postal_address: postaladdress_ebill2wire(input.postal_address),
    }
}
