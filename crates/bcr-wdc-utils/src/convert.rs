// ----- standard library imports
// ----- extra library imports
use bcr_common::wire::{
    bill as wire_bill, contact as wire_contact, identity as wire_identity, quotes as wire_quotes,
};
use bcr_ebill_core::protocol::{
    blockchain::bill::{
        block::ContactType,
        participant::{BillAnonParticipant, BillIdentParticipant, BillParticipant},
        BillToShareWithExternalParty,
    },
    Address, City, Country, Email, Name, PostalAddress, ProtocolValidationError, Zip,
};
use thiserror::Error;
// ----- local imports

// ----- end imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("Url parse {0}")]
    UrlParse(#[from] url::ParseError),
    #[error("ebill parse {0}")]
    EBillParse(#[from] ProtocolValidationError),
}

fn postaladdress_ebill2wire(input: PostalAddress) -> wire_identity::PostalAddress {
    wire_identity::PostalAddress {
        country: input.country.to_string(),
        city: input.city.to_string(),
        zip: input.zip.map(|z| z.to_string()),
        address: input.address.to_string(),
    }
}

fn postaladdress_wire2ebill(input: wire_identity::PostalAddress) -> Result<PostalAddress> {
    let output = PostalAddress {
        country: Country::parse(&input.country)?,
        city: City::new(input.city)?,
        zip: input.zip.map(|z| Zip::new(&z)).transpose()?,
        address: Address::new(input.address)?,
    };
    Ok(output)
}

fn contacttype_ebill2wire(input: ContactType) -> wire_contact::ContactType {
    match input {
        ContactType::Person => wire_contact::ContactType::Person,
        ContactType::Company => wire_contact::ContactType::Company,
        ContactType::Anon => wire_contact::ContactType::Anon,
    }
}

fn contacttype_wire2ebill(input: wire_contact::ContactType) -> ContactType {
    match input {
        wire_contact::ContactType::Person => ContactType::Person,
        wire_contact::ContactType::Company => ContactType::Company,
        wire_contact::ContactType::Anon => ContactType::Anon,
    }
}

pub fn billidentparticipant_ebill2wire(
    input: BillIdentParticipant,
) -> wire_bill::BillIdentParticipant {
    wire_bill::BillIdentParticipant {
        t: contacttype_ebill2wire(input.t),
        name: input.name.to_string(),
        node_id: input.node_id,
        postal_address: postaladdress_ebill2wire(input.postal_address),
        email: input.email.map(|e| e.to_string()),
        nostr_relays: input.nostr_relays,
    }
}

pub fn billidentparticipant_wire2ebill(
    input: wire_bill::BillIdentParticipant,
) -> Result<BillIdentParticipant> {
    let output = BillIdentParticipant {
        t: contacttype_wire2ebill(input.t),
        name: Name::new(input.name)?,
        node_id: input.node_id,
        postal_address: postaladdress_wire2ebill(input.postal_address)?,
        email: input.email.map(|e| Email::new(&e)).transpose()?,
        nostr_relays: input
            .nostr_relays
            .iter()
            .map(|r| r.as_str())
            .map(reqwest::Url::parse)
            .collect::<std::result::Result<Vec<_>, _>>()?,
    };
    Ok(output)
}

fn billanonparticipant_ebill2wire(input: BillAnonParticipant) -> wire_bill::BillAnonParticipant {
    wire_bill::BillAnonParticipant {
        node_id: input.node_id,
        nostr_relays: input.nostr_relays,
    }
}

fn billanonparticipant_wire2ebill(
    input: wire_bill::BillAnonParticipant,
) -> Result<BillAnonParticipant> {
    let output = BillAnonParticipant {
        node_id: input.node_id,
        nostr_relays: input
            .nostr_relays
            .iter()
            .map(|r| r.as_str())
            .map(reqwest::Url::parse)
            .collect::<std::result::Result<Vec<_>, _>>()?,
    };
    Ok(output)
}

pub fn billparticipant_ebill2wire(input: BillParticipant) -> wire_bill::BillParticipant {
    match input {
        BillParticipant::Ident(data) => {
            wire_bill::BillParticipant::Ident(billidentparticipant_ebill2wire(data))
        }
        BillParticipant::Anon(data) => {
            wire_bill::BillParticipant::Anon(billanonparticipant_ebill2wire(data))
        }
    }
}

pub fn billparticipant_wire2ebill(input: wire_bill::BillParticipant) -> Result<BillParticipant> {
    let output = match input {
        wire_bill::BillParticipant::Ident(data) => {
            BillParticipant::Ident(billidentparticipant_wire2ebill(data)?)
        }
        wire_bill::BillParticipant::Anon(data) => {
            BillParticipant::Anon(billanonparticipant_wire2ebill(data)?)
        }
    };
    Ok(output)
}

pub fn sharedbill_ebill2wire(input: BillToShareWithExternalParty) -> wire_quotes::SharedBill {
    wire_quotes::SharedBill {
        bill_id: input.bill_id,
        data: input.data,
        file_urls: input.file_urls,
        hash: input.hash.to_string(),
        signature: input.signature.to_string(),
        receiver: input.receiver.into(),
    }
}
