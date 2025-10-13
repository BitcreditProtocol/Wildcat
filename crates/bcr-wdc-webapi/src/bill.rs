// ----- standard library imports
// ----- extra library imports
use bcr_common::wire::{contact as wire_contact, identity as wire_identity, bill as wire_bill};
pub use bcr_ebill_core::bill::BillId;
pub use bcr_ebill_core::NodeId;
use bcr_ebill_core::{
    bill::{self},
    contact, notification,
    util::date::DateTimeUtc,
};
use bcr_wdc_utils::convert;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
// ----- local imports

// ----- end imports

#[derive(Debug, Serialize, Deserialize)]
pub struct BillsResponse<T: Serialize> {
    pub bills: Vec<T>,
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct BitcreditBill {
    #[schema(value_type=String)]
    pub id: BillId,
    pub participants: BillParticipants,
    pub data: BillData,
    pub status: BillStatus,
    pub current_waiting_state: Option<BillCurrentWaitingState>,
}

impl From<bill::BitcreditBillResult> for BitcreditBill {
    fn from(val: bill::BitcreditBillResult) -> Self {
        BitcreditBill {
            id: val.id,
            participants: val.participants.into(),
            data: val.data.into(),
            status: val.status.into(),
            current_waiting_state: val.current_waiting_state.map(|cws| cws.into()),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub enum BillCurrentWaitingState {
    Sell(BillWaitingForSellState),
    Payment(BillWaitingForPaymentState),
    Recourse(BillWaitingForRecourseState),
}

impl From<bill::BillCurrentWaitingState> for BillCurrentWaitingState {
    fn from(val: bill::BillCurrentWaitingState) -> Self {
        match val {
            bill::BillCurrentWaitingState::Sell(state) => {
                BillCurrentWaitingState::Sell(state.into())
            }
            bill::BillCurrentWaitingState::Payment(state) => {
                BillCurrentWaitingState::Payment(state.into())
            }
            bill::BillCurrentWaitingState::Recourse(state) => {
                BillCurrentWaitingState::Recourse(state.into())
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct BillWaitingForSellState {
    pub time_of_request: u64,
    pub buyer: BillParticipant,
    pub seller: BillParticipant,
    pub currency: String,
    pub sum: String,
    pub link_to_pay: String,
    pub address_to_pay: String,
    pub mempool_link_for_address_to_pay: String,
}

impl From<bill::BillWaitingForSellState> for BillWaitingForSellState {
    fn from(val: bill::BillWaitingForSellState) -> Self {
        BillWaitingForSellState {
            time_of_request: val.payment_data.time_of_request,
            buyer: val.buyer.into(),
            seller: val.seller.into(),
            currency: val.payment_data.currency,
            sum: val.payment_data.sum,
            link_to_pay: val.payment_data.link_to_pay,
            address_to_pay: val.payment_data.address_to_pay,
            mempool_link_for_address_to_pay: val.payment_data.mempool_link_for_address_to_pay,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct BillWaitingForPaymentState {
    pub time_of_request: u64,
    pub payer: BillIdentParticipant,
    pub payee: BillParticipant,
    pub currency: String,
    pub sum: String,
    pub link_to_pay: String,
    pub address_to_pay: String,
    pub mempool_link_for_address_to_pay: String,
}

impl From<bill::BillWaitingForPaymentState> for BillWaitingForPaymentState {
    fn from(val: bill::BillWaitingForPaymentState) -> Self {
        BillWaitingForPaymentState {
            time_of_request: val.payment_data.time_of_request,
            payer: val.payer.into(),
            payee: val.payee.into(),
            currency: val.payment_data.currency,
            sum: val.payment_data.sum,
            link_to_pay: val.payment_data.link_to_pay,
            address_to_pay: val.payment_data.address_to_pay,
            mempool_link_for_address_to_pay: val.payment_data.mempool_link_for_address_to_pay,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct BillWaitingForRecourseState {
    pub time_of_request: u64,
    pub recourser: BillIdentParticipant,
    pub recoursee: BillIdentParticipant,
    pub currency: String,
    pub sum: String,
    pub link_to_pay: String,
    pub address_to_pay: String,
    pub mempool_link_for_address_to_pay: String,
}

impl From<bill::BillWaitingForRecourseState> for BillWaitingForRecourseState {
    fn from(val: bill::BillWaitingForRecourseState) -> Self {
        BillWaitingForRecourseState {
            time_of_request: val.payment_data.time_of_request,
            recourser: val.recourser.into(),
            recoursee: val.recoursee.into(),
            currency: val.payment_data.currency,
            sum: val.payment_data.sum,
            link_to_pay: val.payment_data.link_to_pay,
            address_to_pay: val.payment_data.address_to_pay,
            mempool_link_for_address_to_pay: val.payment_data.mempool_link_for_address_to_pay,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct BillStatus {
    pub acceptance: BillAcceptanceStatus,
    pub payment: BillPaymentStatus,
    pub sell: BillSellStatus,
    pub recourse: BillRecourseStatus,
    pub redeemed_funds_available: bool,
    pub has_requested_funds: bool,
}

impl From<bill::BillStatus> for BillStatus {
    fn from(val: bill::BillStatus) -> Self {
        BillStatus {
            acceptance: val.acceptance.into(),
            payment: val.payment.into(),
            sell: val.sell.into(),
            recourse: val.recourse.into(),
            redeemed_funds_available: val.redeemed_funds_available,
            has_requested_funds: val.has_requested_funds,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct BillAcceptanceStatus {
    pub time_of_request_to_accept: Option<u64>,
    pub requested_to_accept: bool,
    pub accepted: bool,
    pub request_to_accept_timed_out: bool,
    pub rejected_to_accept: bool,
}

impl From<bill::BillAcceptanceStatus> for BillAcceptanceStatus {
    fn from(val: bill::BillAcceptanceStatus) -> Self {
        BillAcceptanceStatus {
            time_of_request_to_accept: val.time_of_request_to_accept,
            requested_to_accept: val.requested_to_accept,
            accepted: val.accepted,
            request_to_accept_timed_out: val.request_to_accept_timed_out,
            rejected_to_accept: val.rejected_to_accept,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct BillPaymentStatus {
    pub time_of_request_to_pay: Option<u64>,
    pub requested_to_pay: bool,
    pub paid: bool,
    pub request_to_pay_timed_out: bool,
    pub rejected_to_pay: bool,
}

impl From<bill::BillPaymentStatus> for BillPaymentStatus {
    fn from(val: bill::BillPaymentStatus) -> Self {
        BillPaymentStatus {
            time_of_request_to_pay: val.time_of_request_to_pay,
            requested_to_pay: val.requested_to_pay,
            paid: val.paid,
            request_to_pay_timed_out: val.request_to_pay_timed_out,
            rejected_to_pay: val.rejected_to_pay,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct BillSellStatus {
    pub time_of_last_offer_to_sell: Option<u64>,
    pub sold: bool,
    pub offered_to_sell: bool,
    pub offer_to_sell_timed_out: bool,
    pub rejected_offer_to_sell: bool,
}

impl From<bill::BillSellStatus> for BillSellStatus {
    fn from(val: bill::BillSellStatus) -> Self {
        BillSellStatus {
            time_of_last_offer_to_sell: val.time_of_last_offer_to_sell,
            sold: val.sold,
            offered_to_sell: val.offered_to_sell,
            offer_to_sell_timed_out: val.offer_to_sell_timed_out,
            rejected_offer_to_sell: val.rejected_offer_to_sell,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct BillRecourseStatus {
    pub time_of_last_request_to_recourse: Option<u64>,
    pub recoursed: bool,
    pub requested_to_recourse: bool,
    pub request_to_recourse_timed_out: bool,
    pub rejected_request_to_recourse: bool,
}

impl From<bill::BillRecourseStatus> for BillRecourseStatus {
    fn from(val: bill::BillRecourseStatus) -> Self {
        BillRecourseStatus {
            time_of_last_request_to_recourse: val.time_of_last_request_to_recourse,
            recoursed: val.recoursed,
            requested_to_recourse: val.requested_to_recourse,
            request_to_recourse_timed_out: val.request_to_recourse_timed_out,
            rejected_request_to_recourse: val.rejected_request_to_recourse,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct BillData {
    pub language: String,
    pub time_of_drawing: u64,
    pub issue_date: String,
    pub time_of_maturity: u64,
    pub maturity_date: String,
    pub country_of_issuing: String,
    pub city_of_issuing: String,
    pub country_of_payment: String,
    pub city_of_payment: String,
    pub currency: String,
    pub sum: String,
    pub files: Vec<wire_identity::File>,
    pub active_notification: Option<Notification>,
}

impl From<bill::BillData> for BillData {
    fn from(val: bill::BillData) -> Self {
        BillData {
            language: val.language,
            time_of_drawing: val.time_of_drawing,
            issue_date: val.issue_date,
            time_of_maturity: val.time_of_maturity,
            maturity_date: val.maturity_date,
            country_of_issuing: val.country_of_issuing,
            city_of_issuing: val.city_of_issuing,
            country_of_payment: val.country_of_payment,
            city_of_payment: val.city_of_payment,
            currency: val.currency,
            sum: val.sum,
            files: val
                .files
                .into_iter()
                .map(convert::file_ebill2wire)
                .collect(),
            active_notification: val.active_notification.map(|an| an.into()),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct BillParticipants {
    pub drawee: BillIdentParticipant,
    pub drawer: BillIdentParticipant,
    pub payee: BillParticipant,
    pub endorsee: Option<BillParticipant>,
    pub endorsements_count: u64,
    #[schema(value_type=Vec<String>)]
    pub all_participant_node_ids: Vec<NodeId>,
}

impl From<bill::BillParticipants> for BillParticipants {
    fn from(val: bill::BillParticipants) -> Self {
        BillParticipants {
            drawee: val.drawee.into(),
            drawer: val.drawer.into(),
            payee: val.payee.into(),
            endorsee: val.endorsee.map(|e| e.into()),
            endorsements_count: val.endorsements_count,
            all_participant_node_ids: val.all_participant_node_ids,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize, ToSchema)]
pub enum BillParticipant {
    Anon(BillAnonParticipant),
    Ident(BillIdentParticipant),
}

impl BillParticipant {
    pub fn node_id(&self) -> NodeId {
        match self {
            BillParticipant::Ident(data) => data.node_id.clone(),
            BillParticipant::Anon(data) => data.node_id.clone(),
        }
    }
}

impl From<contact::BillParticipant> for BillParticipant {
    fn from(val: contact::BillParticipant) -> Self {
        match val {
            contact::BillParticipant::Ident(data) => BillParticipant::Ident(data.into()),
            contact::BillParticipant::Anon(data) => BillParticipant::Anon(data.into()),
        }
    }
}

impl From<BillParticipant> for contact::BillParticipant {
    fn from(val: BillParticipant) -> Self {
        match val {
            BillParticipant::Ident(data) => contact::BillParticipant::Ident(data.into()),
            BillParticipant::Anon(data) => contact::BillParticipant::Anon(data.into()),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize, ToSchema)]
pub struct BillAnonParticipant {
    #[schema(value_type=String)]
    pub node_id: NodeId,
    pub email: Option<String>,
    pub nostr_relays: Vec<String>,
}

impl From<contact::BillAnonParticipant> for BillAnonParticipant {
    fn from(val: contact::BillAnonParticipant) -> Self {
        BillAnonParticipant {
            node_id: val.node_id,
            email: val.email,
            nostr_relays: val.nostr_relays,
        }
    }
}

impl From<BillAnonParticipant> for contact::BillAnonParticipant {
    fn from(val: BillAnonParticipant) -> Self {
        contact::BillAnonParticipant {
            node_id: val.node_id,
            email: val.email,
            nostr_relays: val.nostr_relays,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize, ToSchema)]
pub struct BillIdentParticipant {
    #[serde(rename = "type")]
    pub t: wire_contact::ContactType,
    #[schema(value_type=String)]
    pub node_id: NodeId,
    pub name: String,
    #[serde(flatten)]
    pub postal_address: wire_identity::PostalAddress,
    pub email: Option<String>,
    pub nostr_relays: Vec<String>,
}

impl From<contact::BillIdentParticipant> for BillIdentParticipant {
    fn from(val: contact::BillIdentParticipant) -> Self {
        BillIdentParticipant {
            t: convert::contacttype_ebill2wire(val.t),
            name: val.name,
            node_id: val.node_id,
            postal_address: convert::postaladdress_ebill2wire(val.postal_address),
            email: val.email,
            nostr_relays: val.nostr_relays,
        }
    }
}

impl From<BillIdentParticipant> for contact::BillIdentParticipant {
    fn from(val: BillIdentParticipant) -> Self {
        contact::BillIdentParticipant {
            t: convert::contacttype_wire2ebill(val.t),
            name: val.name,
            node_id: val.node_id,
            postal_address: convert::postaladdress_wire2ebill(val.postal_address),
            email: val.email,
            nostr_relays: val.nostr_relays,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Notification {
    pub id: String,
    #[schema(value_type=Option<String>)]
    pub node_id: Option<NodeId>,
    pub notification_type: wire_bill::NotificationType,
    pub reference_id: Option<String>,
    pub description: String,
    #[schema(value_type = chrono::DateTime<chrono::Utc>)]
    pub datetime: DateTimeUtc,
    pub active: bool,
    pub payload: Option<serde_json::Value>,
}

impl From<notification::Notification> for Notification {
    fn from(val: notification::Notification) -> Self {
        Notification {
            id: val.id,
            node_id: val.node_id,
            notification_type: convert::notificationtype_ebill2wire(val.notification_type),
            reference_id: val.reference_id,
            description: val.description,
            datetime: val.datetime,
            active: val.active,
            payload: val.payload,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestToPayBitcreditBillPayload {
    pub bill_id: BillId,
    pub currency: String,
}
