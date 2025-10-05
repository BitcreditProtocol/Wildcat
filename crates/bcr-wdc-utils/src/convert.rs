// ----- standard library imports
// ----- extra library imports
use bcr_common::wire::{contact as wire_contact, identity as wire_identity};
use bcr_ebill_core::{self as ebill_core, contact as ebill_contact, identity as ebill_identity};
use thiserror::Error;
// ----- local imports

// ----- end imports

#[derive(Debug, Error)]
pub enum Error {}

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
