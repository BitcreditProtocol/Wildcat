// ----- standard library imports
// ----- extra library imports
use bcr_common::wire::{bill as wire_bill, identity as wire_identity};
pub use bcr_ebill_core::bill::BillId;
use bcr_ebill_core::bill::{self};
pub use bcr_ebill_core::NodeId;
use bcr_wdc_utils::convert;
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
    pub buyer: wire_bill::BillParticipant,
    pub seller: wire_bill::BillParticipant,
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
            buyer: convert::billparticipant_ebill2wire(val.buyer),
            seller: convert::billparticipant_ebill2wire(val.seller),
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
    pub payer: wire_bill::BillIdentParticipant,
    pub payee: wire_bill::BillParticipant,
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
            payer: convert::billidentparticipant_ebill2wire(val.payer),
            payee: convert::billparticipant_ebill2wire(val.payee),
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
    pub recourser: wire_bill::BillIdentParticipant,
    pub recoursee: wire_bill::BillIdentParticipant,
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
            recourser: convert::billidentparticipant_ebill2wire(val.recourser),
            recoursee: convert::billidentparticipant_ebill2wire(val.recoursee),
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
    pub active_notification: Option<wire_bill::Notification>,
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
            active_notification: val
                .active_notification
                .map(convert::notification_ebill2wire),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct BillParticipants {
    pub drawee: wire_bill::BillIdentParticipant,
    pub drawer: wire_bill::BillIdentParticipant,
    pub payee: wire_bill::BillParticipant,
    pub endorsee: Option<wire_bill::BillParticipant>,
    pub endorsements_count: u64,
    #[schema(value_type=Vec<String>)]
    pub all_participant_node_ids: Vec<NodeId>,
}

impl From<bill::BillParticipants> for BillParticipants {
    fn from(val: bill::BillParticipants) -> Self {
        BillParticipants {
            drawee: convert::billidentparticipant_ebill2wire(val.drawee),
            drawer: convert::billidentparticipant_ebill2wire(val.drawer),
            payee: convert::billparticipant_ebill2wire(val.payee),
            endorsee: val.endorsee.map(convert::billparticipant_ebill2wire),
            endorsements_count: val.endorsements_count,
            all_participant_node_ids: val.all_participant_node_ids,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestToPayBitcreditBillPayload {
    pub bill_id: BillId,
    pub currency: String,
}
