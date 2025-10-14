// ----- standard library imports
// ----- extra library imports
use bcr_common::wire::bill as wire_bill;
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
    pub participants: wire_bill::BillParticipants,
    pub data: wire_bill::BillData,
    pub status: wire_bill::BillStatus,
    pub current_waiting_state: Option<BillCurrentWaitingState>,
}

impl TryFrom<bill::BitcreditBillResult> for BitcreditBill {
    type Error = convert::Error;
    fn try_from(val: bill::BitcreditBillResult) -> std::result::Result<Self, Self::Error> {
        let retv = Self {
            id: val.id,
            participants: convert::billparticipants_ebill2wire(val.participants),
            data: convert::billdata_ebill2wire(val.data)?,
            status: convert::billstatus_ebill2wire(val.status),
            current_waiting_state: val.current_waiting_state.map(|cws| cws.into()),
        };
        Ok(retv)
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

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestToPayBitcreditBillPayload {
    pub bill_id: BillId,
    pub currency: String,
}
