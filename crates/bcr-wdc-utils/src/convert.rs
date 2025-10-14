// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use bcr_common::{
    core,
    wire::{bill as wire_bill, contact as wire_contact, identity as wire_identity},
};
use bcr_ebill_core::{
    self as ebill_core, bill as ebill_bill, contact as ebill_contact, identity as ebill_identity,
    notification as ebill_notification,
};
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

pub fn nodeid_wire2ebill(input: core::NodeId) -> ebill_core::NodeId {
    ebill_core::NodeId::new(input.pub_key(), input.network())
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

fn lightbillidentparticipantwithaddress_ebill2wire(
    input: ebill_contact::LightBillIdentParticipantWithAddress,
) -> wire_bill::LightBillIdentParticipantWithAddress {
    wire_bill::LightBillIdentParticipantWithAddress {
        t: contacttype_ebill2wire(input.t),
        name: input.name,
        node_id: nodeid_ebill2wire(input.node_id),
        postal_address: postaladdress_ebill2wire(input.postal_address),
    }
}

fn lightbillidentparticipant_ebill2wire(
    input: ebill_contact::LightBillIdentParticipant,
) -> wire_bill::LightBillIdentParticipant {
    wire_bill::LightBillIdentParticipant {
        t: contacttype_ebill2wire(input.t),
        name: input.name,
        node_id: nodeid_ebill2wire(input.node_id),
    }
}

fn lightbillanonparticipant_ebill2wire(
    input: ebill_contact::LightBillAnonParticipant,
) -> wire_bill::LightBillAnonParticipant {
    wire_bill::LightBillAnonParticipant {
        node_id: nodeid_ebill2wire(input.node_id),
    }
}

fn lightbillparticipant_ebill2wire(
    input: ebill_contact::LightBillParticipant,
) -> wire_bill::LightBillParticipant {
    match input {
        ebill_contact::LightBillParticipant::Ident(data) => wire_bill::LightBillParticipant::Ident(
            lightbillidentparticipantwithaddress_ebill2wire(data),
        ),
        ebill_contact::LightBillParticipant::Anon(data) => {
            wire_bill::LightBillParticipant::Anon(lightbillanonparticipant_ebill2wire(data))
        }
    }
}

fn lightsignedby_ebill2wire(input: ebill_bill::LightSignedBy) -> wire_bill::LightSignedBy {
    wire_bill::LightSignedBy {
        data: lightbillparticipant_ebill2wire(input.data),
        signatory: input.signatory.map(lightbillidentparticipant_ebill2wire),
    }
}

pub fn endorsement_ebill2wire(input: ebill_bill::Endorsement) -> wire_bill::Endorsement {
    wire_bill::Endorsement {
        pay_to_the_order_of: lightbillidentparticipantwithaddress_ebill2wire(
            input.pay_to_the_order_of,
        ),
        signed: lightsignedby_ebill2wire(input.signed),
        signing_timestamp: input.signing_timestamp,
        signing_address: input.signing_address.map(postaladdress_ebill2wire),
    }
}

pub fn billcombinedbitcoinkey_ebill2wire(
    input: ebill_bill::BillCombinedBitcoinKey,
) -> wire_bill::BillCombinedBitcoinKey {
    wire_bill::BillCombinedBitcoinKey {
        private_descriptor: input.private_descriptor,
    }
}

pub fn notificationtype_ebill2wire(
    input: ebill_notification::NotificationType,
) -> wire_bill::NotificationType {
    match input {
        ebill_notification::NotificationType::Bill => wire_bill::NotificationType::Bill,
        ebill_notification::NotificationType::General => wire_bill::NotificationType::General,
    }
}

pub fn notification_ebill2wire(input: ebill_notification::Notification) -> wire_bill::Notification {
    wire_bill::Notification {
        id: input.id,
        node_id: input.node_id.map(nodeid_ebill2wire),
        notification_type: notificationtype_ebill2wire(input.notification_type),
        reference_id: input.reference_id,
        description: input.description,
        datetime: input.datetime,
        active: input.active,
        payload: input.payload,
    }
}

pub fn billidentparticipant_ebill2wire(
    input: ebill_contact::BillIdentParticipant,
) -> wire_bill::BillIdentParticipant {
    wire_bill::BillIdentParticipant {
        t: contacttype_ebill2wire(input.t),
        name: input.name,
        node_id: nodeid_ebill2wire(input.node_id),
        postal_address: postaladdress_ebill2wire(input.postal_address),
        email: input.email,
        nostr_relays: input.nostr_relays,
    }
}

pub fn billidentparticipant_wire2ebill(
    input: wire_bill::BillIdentParticipant,
) -> ebill_contact::BillIdentParticipant {
    ebill_contact::BillIdentParticipant {
        t: contacttype_wire2ebill(input.t),
        name: input.name,
        node_id: nodeid_wire2ebill(input.node_id),
        postal_address: postaladdress_wire2ebill(input.postal_address),
        email: input.email,
        nostr_relays: input.nostr_relays,
    }
}

pub fn billanonparticipant_ebill2wire(
    input: ebill_contact::BillAnonParticipant,
) -> wire_bill::BillAnonParticipant {
    wire_bill::BillAnonParticipant {
        node_id: nodeid_ebill2wire(input.node_id),
        email: input.email,
        nostr_relays: input.nostr_relays,
    }
}
pub fn billanonparticipant_wire2ebill(
    input: wire_bill::BillAnonParticipant,
) -> ebill_contact::BillAnonParticipant {
    ebill_contact::BillAnonParticipant {
        node_id: nodeid_wire2ebill(input.node_id),
        email: input.email,
        nostr_relays: input.nostr_relays,
    }
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
) -> ebill_contact::BillParticipant {
    match input {
        wire_bill::BillParticipant::Ident(data) => {
            ebill_contact::BillParticipant::Ident(billidentparticipant_wire2ebill(data))
        }
        wire_bill::BillParticipant::Anon(data) => {
            ebill_contact::BillParticipant::Anon(billanonparticipant_wire2ebill(data))
        }
    }
}

pub fn billparticipants_ebill2wire(
    input: ebill_bill::BillParticipants,
) -> wire_bill::BillParticipants {
    wire_bill::BillParticipants {
        drawee: billidentparticipant_ebill2wire(input.drawee),
        drawer: billidentparticipant_ebill2wire(input.drawer),
        payee: billparticipant_ebill2wire(input.payee),
        endorsee: input.endorsee.map(billparticipant_ebill2wire),
        endorsements_count: input.endorsements_count,
        all_participant_node_ids: input
            .all_participant_node_ids
            .into_iter()
            .map(nodeid_ebill2wire)
            .collect(),
    }
}

pub fn billdata_ebill2wire(input: ebill_bill::BillData) -> Result<wire_bill::BillData> {
    let issue_date = chrono::NaiveDate::from_str(&input.issue_date)?;
    let maturity_date = chrono::NaiveDate::from_str(&input.maturity_date)?;
    let output = wire_bill::BillData {
        language: input.language,
        time_of_drawing: input.time_of_drawing,
        issue_date,
        time_of_maturity: input.time_of_maturity,
        maturity_date,
        country_of_issuing: input.country_of_issuing,
        city_of_issuing: input.city_of_issuing,
        country_of_payment: input.country_of_payment,
        city_of_payment: input.city_of_payment,
        currency: input.currency,
        sum: input.sum,
        files: input.files.into_iter().map(file_ebill2wire).collect(),
        active_notification: input.active_notification.map(notification_ebill2wire),
    };
    Ok(output)
}

pub fn billpaymentstatus_ebill2wire(
    input: ebill_bill::BillPaymentStatus,
) -> wire_bill::BillPaymentStatus {
    wire_bill::BillPaymentStatus {
        rejected_to_pay: input.rejected_to_pay,
        requested_to_pay: input.requested_to_pay,
        request_to_pay_timed_out: input.request_to_pay_timed_out,
        time_of_request_to_pay: input.time_of_request_to_pay,
        paid: input.paid,
    }
}

pub fn billstatus_ebill2wire(input: ebill_bill::BillStatus) -> wire_bill::BillStatus {
    let acceptance = wire_bill::BillAcceptanceStatus {
        time_of_request_to_accept: input.acceptance.time_of_request_to_accept,
        accepted: input.acceptance.accepted,
        rejected_to_accept: input.acceptance.rejected_to_accept,
        requested_to_accept: input.acceptance.requested_to_accept,
        request_to_accept_timed_out: input.acceptance.request_to_accept_timed_out,
    };
    let payment = billpaymentstatus_ebill2wire(input.payment);
    let sell = wire_bill::BillSellStatus {
        offered_to_sell: input.sell.offered_to_sell,
        offer_to_sell_timed_out: input.sell.offer_to_sell_timed_out,
        rejected_offer_to_sell: input.sell.rejected_offer_to_sell,
        sold: input.sell.sold,
        time_of_last_offer_to_sell: input.sell.time_of_last_offer_to_sell,
    };
    let recourse = wire_bill::BillRecourseStatus {
        recoursed: input.recourse.recoursed,
        requested_to_recourse: input.recourse.requested_to_recourse,
        request_to_recourse_timed_out: input.recourse.request_to_recourse_timed_out,
        rejected_request_to_recourse: input.recourse.rejected_request_to_recourse,
        time_of_last_request_to_recourse: input.recourse.time_of_last_request_to_recourse,
    };
    wire_bill::BillStatus {
        acceptance,
        payment,
        sell,
        recourse,
        redeemed_funds_available: input.redeemed_funds_available,
        has_requested_funds: input.has_requested_funds,
    }
}

pub fn billwaitingforpaymentstate_ebill2wire(
    input: ebill_bill::BillWaitingForPaymentState,
) -> wire_bill::BillWaitingForPaymentState {
    wire_bill::BillWaitingForPaymentState {
        address_to_pay: input.payment_data.address_to_pay,
        currency: input.payment_data.currency,
        link_to_pay: input.payment_data.link_to_pay,
        mempool_link_for_address_to_pay: input.payment_data.mempool_link_for_address_to_pay,
        payee: billparticipant_ebill2wire(input.payee),
        payer: billidentparticipant_ebill2wire(input.payer),
        time_of_request: input.payment_data.time_of_request,
        sum: input.payment_data.sum,
    }
}

pub fn billcurrentwaitingstate_ebill2wire(
    input: ebill_bill::BillCurrentWaitingState,
) -> wire_bill::BillCurrentWaitingState {
    match input {
        ebill_bill::BillCurrentWaitingState::Sell(state) => {
            let state = wire_bill::BillWaitingForSellState {
                address_to_pay: state.payment_data.address_to_pay,
                buyer: billparticipant_ebill2wire(state.buyer),
                currency: state.payment_data.currency,
                link_to_pay: state.payment_data.link_to_pay,
                mempool_link_for_address_to_pay: state.payment_data.mempool_link_for_address_to_pay,
                seller: billparticipant_ebill2wire(state.seller),
                sum: state.payment_data.sum,
                time_of_request: state.payment_data.time_of_request,
            };
            wire_bill::BillCurrentWaitingState::Sell(state)
        }
        ebill_bill::BillCurrentWaitingState::Payment(state) => {
            let state = billwaitingforpaymentstate_ebill2wire(state);
            wire_bill::BillCurrentWaitingState::Payment(state)
        }
        ebill_bill::BillCurrentWaitingState::Recourse(state) => {
            let state = wire_bill::BillWaitingForRecourseState {
                address_to_pay: state.payment_data.address_to_pay,
                currency: state.payment_data.currency,
                link_to_pay: state.payment_data.link_to_pay,
                time_of_request: state.payment_data.time_of_request,
                mempool_link_for_address_to_pay: state.payment_data.mempool_link_for_address_to_pay,
                recourser: billidentparticipant_ebill2wire(state.recourser),
                recoursee: billidentparticipant_ebill2wire(state.recoursee),
                sum: state.payment_data.sum,
            };
            wire_bill::BillCurrentWaitingState::Recourse(state)
        }
    }
}
