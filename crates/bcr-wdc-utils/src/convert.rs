// ----- standard library imports
// ----- extra library imports
use bcr_common::{
    core,
    wire::{
        bill as wire_bill, contact as wire_contact, identity as wire_identity, keys as wire_keys,
        quotes as wire_quotes,
    },
};
use bcr_ebill_core::{
    self as ebill_core, address::Address, blockchain::bill::BillToShareWithExternalParty,
    city::City, contact as ebill_contact, country::Country, email::Email, name::Name, zip::Zip,
};
use clwdr_client::model as clwdr_model;
use thiserror::Error;
// ----- local imports

// ----- end imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("Url parse {0}")]
    UrlParse(#[from] url::ParseError),
    #[error("ebill parse {0}")]
    EBillParse(#[from] bcr_ebill_core::ValidationError),
}

fn postaladdress_ebill2wire(input: ebill_core::PostalAddress) -> wire_identity::PostalAddress {
    wire_identity::PostalAddress {
        country: input.country.to_string(),
        city: input.city.to_string(),
        zip: input.zip.map(|z| z.to_string()),
        address: input.address.to_string(),
    }
}

fn postaladdress_wire2ebill(
    input: wire_identity::PostalAddress,
) -> Result<ebill_core::PostalAddress> {
    let output = ebill_core::PostalAddress {
        country: Country::parse(&input.country)?,
        city: City::new(input.city)?,
        zip: input.zip.map(|z| Zip::new(&z)).transpose()?,
        address: Address::new(input.address)?,
    };
    Ok(output)
}

fn contacttype_ebill2wire(input: ebill_contact::ContactType) -> wire_contact::ContactType {
    match input {
        ebill_contact::ContactType::Person => wire_contact::ContactType::Person,
        ebill_contact::ContactType::Company => wire_contact::ContactType::Company,
        ebill_contact::ContactType::Anon => wire_contact::ContactType::Anon,
    }
}

fn contacttype_wire2ebill(input: wire_contact::ContactType) -> ebill_contact::ContactType {
    match input {
        wire_contact::ContactType::Person => ebill_contact::ContactType::Person,
        wire_contact::ContactType::Company => ebill_contact::ContactType::Company,
        wire_contact::ContactType::Anon => ebill_contact::ContactType::Anon,
    }
}

fn nodeid_ebill2wire(input: ebill_core::NodeId) -> core::NodeId {
    core::NodeId::new(input.pub_key(), input.network())
}

pub fn nodeid_wire2ebill(input: core::NodeId) -> ebill_core::NodeId {
    ebill_core::NodeId::new(input.pub_key(), input.network())
}

pub fn billidentparticipant_ebill2wire(
    input: ebill_contact::BillIdentParticipant,
) -> wire_bill::BillIdentParticipant {
    wire_bill::BillIdentParticipant {
        t: contacttype_ebill2wire(input.t),
        name: input.name.to_string(),
        node_id: nodeid_ebill2wire(input.node_id),
        postal_address: postaladdress_ebill2wire(input.postal_address),
        email: input.email.map(|e| e.to_string()),
        nostr_relays: input.nostr_relays,
    }
}

pub fn billidentparticipant_wire2ebill(
    input: wire_bill::BillIdentParticipant,
) -> Result<ebill_contact::BillIdentParticipant> {
    let output = ebill_contact::BillIdentParticipant {
        t: contacttype_wire2ebill(input.t),
        name: Name::new(input.name)?,
        node_id: nodeid_wire2ebill(input.node_id),
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

fn billanonparticipant_ebill2wire(
    input: ebill_contact::BillAnonParticipant,
) -> wire_bill::BillAnonParticipant {
    wire_bill::BillAnonParticipant {
        node_id: nodeid_ebill2wire(input.node_id),
        email: input.email.map(|e| e.to_string()),
        nostr_relays: input.nostr_relays,
    }
}

fn billanonparticipant_wire2ebill(
    input: wire_bill::BillAnonParticipant,
) -> Result<ebill_contact::BillAnonParticipant> {
    let output = ebill_contact::BillAnonParticipant {
        node_id: nodeid_wire2ebill(input.node_id),
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

pub fn billparticipant_ebill2wire(
    input: ebill_contact::BillParticipant,
) -> wire_bill::BillParticipant {
    match input {
        ebill_contact::BillParticipant::Ident(data) => {
            wire_bill::BillParticipant::Ident(billidentparticipant_ebill2wire(data))
        }
        ebill_contact::BillParticipant::Anon(data) => {
            wire_bill::BillParticipant::Anon(billanonparticipant_ebill2wire(data))
        }
    }
}

pub fn billparticipant_wire2ebill(
    input: wire_bill::BillParticipant,
) -> Result<ebill_contact::BillParticipant> {
    let output = match input {
        wire_bill::BillParticipant::Ident(data) => {
            ebill_contact::BillParticipant::Ident(billidentparticipant_wire2ebill(data)?)
        }
        wire_bill::BillParticipant::Anon(data) => {
            ebill_contact::BillParticipant::Anon(billanonparticipant_wire2ebill(data)?)
        }
    };
    Ok(output)
}

pub fn sharedbill_ebill2wire(input: BillToShareWithExternalParty) -> wire_quotes::SharedBill {
    wire_quotes::SharedBill {
        bill_id: input.bill_id,
        data: input.data,
        file_urls: input.file_urls,
        hash: input.hash,
        signature: input.signature,
        receiver: input.receiver.into(),
    }
}

pub fn prooffingerprint_wire2clowder(
    input: wire_keys::ProofFingerprint,
) -> clwdr_model::ProofFingerprint {
    let amount = cashu::Amount::from(input.amount);
    clwdr_model::ProofFingerprint {
        amount,
        keyset_id: input.keyset_id,
        c: input.c,
        y: input.y,
        dleq: input.dleq,
        witness: input.witness,
    }
}
