// ----- standard library imports
// ----- extra library imports
use bcr_ebill_api::{
    data::{
        bill::{
            BillAcceptanceStatus, BillCombinedBitcoinKey, BillCurrentWaitingState, BillData,
            BillParticipants, BillPaymentStatus, BillRecourseStatus, BillSellStatus, BillStatus,
            BillWaitingForPaymentState, BillWaitingForRecourseState, BillWaitingForSellState,
            BitcreditBillResult,
        },
        contact::{BillAnonParticipant, BillIdentParticipant, BillParticipant},
        notification::{Notification, NotificationType},
    },
    util::date::DateTimeUtc,
};
use serde::{Deserialize, Serialize};
// ----- local imports
use crate::{
    contact::ContactTypeWeb,
    identity::{FileWeb, PostalAddressWeb},
};
// ----- end imports

#[derive(Debug, Serialize)]
pub struct BillsResponse<T: Serialize> {
    pub bills: Vec<T>,
}

#[derive(Debug, Serialize, Clone)]
pub struct BitcreditBillWeb {
    pub id: String,
    pub participants: BillParticipantsWeb,
    pub data: BillDataWeb,
    pub status: BillStatusWeb,
    pub current_waiting_state: Option<BillCurrentWaitingStateWeb>,
}

impl From<BitcreditBillResult> for BitcreditBillWeb {
    fn from(val: BitcreditBillResult) -> Self {
        BitcreditBillWeb {
            id: val.id,
            participants: val.participants.into(),
            data: val.data.into(),
            status: val.status.into(),
            current_waiting_state: val.current_waiting_state.map(|cws| cws.into()),
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub enum BillCurrentWaitingStateWeb {
    Sell(BillWaitingForSellStateWeb),
    Payment(BillWaitingForPaymentStateWeb),
    Recourse(BillWaitingForRecourseStateWeb),
}

impl From<BillCurrentWaitingState> for BillCurrentWaitingStateWeb {
    fn from(val: BillCurrentWaitingState) -> Self {
        match val {
            BillCurrentWaitingState::Sell(state) => BillCurrentWaitingStateWeb::Sell(state.into()),
            BillCurrentWaitingState::Payment(state) => {
                BillCurrentWaitingStateWeb::Payment(state.into())
            }
            BillCurrentWaitingState::Recourse(state) => {
                BillCurrentWaitingStateWeb::Recourse(state.into())
            }
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct BillWaitingForSellStateWeb {
    pub time_of_request: u64,
    pub buyer: BillParticipantWeb,
    pub seller: BillParticipantWeb,
    pub currency: String,
    pub sum: String,
    pub link_to_pay: String,
    pub address_to_pay: String,
    pub mempool_link_for_address_to_pay: String,
}

impl From<BillWaitingForSellState> for BillWaitingForSellStateWeb {
    fn from(val: BillWaitingForSellState) -> Self {
        BillWaitingForSellStateWeb {
            time_of_request: val.time_of_request,
            buyer: val.buyer.into(),
            seller: val.seller.into(),
            currency: val.currency,
            sum: val.sum,
            link_to_pay: val.link_to_pay,
            address_to_pay: val.address_to_pay,
            mempool_link_for_address_to_pay: val.mempool_link_for_address_to_pay,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct BillWaitingForPaymentStateWeb {
    pub time_of_request: u64,
    pub payer: BillIdentParticipantWeb,
    pub payee: BillParticipantWeb,
    pub currency: String,
    pub sum: String,
    pub link_to_pay: String,
    pub address_to_pay: String,
    pub mempool_link_for_address_to_pay: String,
}

impl From<BillWaitingForPaymentState> for BillWaitingForPaymentStateWeb {
    fn from(val: BillWaitingForPaymentState) -> Self {
        BillWaitingForPaymentStateWeb {
            time_of_request: val.time_of_request,
            payer: val.payer.into(),
            payee: val.payee.into(),
            currency: val.currency,
            sum: val.sum,
            link_to_pay: val.link_to_pay,
            address_to_pay: val.address_to_pay,
            mempool_link_for_address_to_pay: val.mempool_link_for_address_to_pay,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct BillWaitingForRecourseStateWeb {
    pub time_of_request: u64,
    pub recourser: BillIdentParticipantWeb,
    pub recoursee: BillIdentParticipantWeb,
    pub currency: String,
    pub sum: String,
    pub link_to_pay: String,
    pub address_to_pay: String,
    pub mempool_link_for_address_to_pay: String,
}

impl From<BillWaitingForRecourseState> for BillWaitingForRecourseStateWeb {
    fn from(val: BillWaitingForRecourseState) -> Self {
        BillWaitingForRecourseStateWeb {
            time_of_request: val.time_of_request,
            recourser: val.recourser.into(),
            recoursee: val.recoursee.into(),
            currency: val.currency,
            sum: val.sum,
            link_to_pay: val.link_to_pay,
            address_to_pay: val.address_to_pay,
            mempool_link_for_address_to_pay: val.mempool_link_for_address_to_pay,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct BillStatusWeb {
    pub acceptance: BillAcceptanceStatusWeb,
    pub payment: BillPaymentStatusWeb,
    pub sell: BillSellStatusWeb,
    pub recourse: BillRecourseStatusWeb,
    pub redeemed_funds_available: bool,
    pub has_requested_funds: bool,
}

impl From<BillStatus> for BillStatusWeb {
    fn from(val: BillStatus) -> Self {
        BillStatusWeb {
            acceptance: val.acceptance.into(),
            payment: val.payment.into(),
            sell: val.sell.into(),
            recourse: val.recourse.into(),
            redeemed_funds_available: val.redeemed_funds_available,
            has_requested_funds: val.has_requested_funds,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct BillAcceptanceStatusWeb {
    pub time_of_request_to_accept: Option<u64>,
    pub requested_to_accept: bool,
    pub accepted: bool,
    pub request_to_accept_timed_out: bool,
    pub rejected_to_accept: bool,
}

impl From<BillAcceptanceStatus> for BillAcceptanceStatusWeb {
    fn from(val: BillAcceptanceStatus) -> Self {
        BillAcceptanceStatusWeb {
            time_of_request_to_accept: val.time_of_request_to_accept,
            requested_to_accept: val.requested_to_accept,
            accepted: val.accepted,
            request_to_accept_timed_out: val.request_to_accept_timed_out,
            rejected_to_accept: val.rejected_to_accept,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct BillPaymentStatusWeb {
    pub time_of_request_to_pay: Option<u64>,
    pub requested_to_pay: bool,
    pub paid: bool,
    pub request_to_pay_timed_out: bool,
    pub rejected_to_pay: bool,
}

impl From<BillPaymentStatus> for BillPaymentStatusWeb {
    fn from(val: BillPaymentStatus) -> Self {
        BillPaymentStatusWeb {
            time_of_request_to_pay: val.time_of_request_to_pay,
            requested_to_pay: val.requested_to_pay,
            paid: val.paid,
            request_to_pay_timed_out: val.request_to_pay_timed_out,
            rejected_to_pay: val.rejected_to_pay,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct BillSellStatusWeb {
    pub time_of_last_offer_to_sell: Option<u64>,
    pub sold: bool,
    pub offered_to_sell: bool,
    pub offer_to_sell_timed_out: bool,
    pub rejected_offer_to_sell: bool,
}

impl From<BillSellStatus> for BillSellStatusWeb {
    fn from(val: BillSellStatus) -> Self {
        BillSellStatusWeb {
            time_of_last_offer_to_sell: val.time_of_last_offer_to_sell,
            sold: val.sold,
            offered_to_sell: val.offered_to_sell,
            offer_to_sell_timed_out: val.offer_to_sell_timed_out,
            rejected_offer_to_sell: val.rejected_offer_to_sell,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct BillRecourseStatusWeb {
    pub time_of_last_request_to_recourse: Option<u64>,
    pub recoursed: bool,
    pub requested_to_recourse: bool,
    pub request_to_recourse_timed_out: bool,
    pub rejected_request_to_recourse: bool,
}

impl From<BillRecourseStatus> for BillRecourseStatusWeb {
    fn from(val: BillRecourseStatus) -> Self {
        BillRecourseStatusWeb {
            time_of_last_request_to_recourse: val.time_of_last_request_to_recourse,
            recoursed: val.recoursed,
            requested_to_recourse: val.requested_to_recourse,
            request_to_recourse_timed_out: val.request_to_recourse_timed_out,
            rejected_request_to_recourse: val.rejected_request_to_recourse,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct BillDataWeb {
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
    pub files: Vec<FileWeb>,
    pub active_notification: Option<NotificationWeb>,
}

impl From<BillData> for BillDataWeb {
    fn from(val: BillData) -> Self {
        BillDataWeb {
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
            files: val.files.into_iter().map(|f| f.into()).collect(),
            active_notification: val.active_notification.map(|an| an.into()),
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct BillParticipantsWeb {
    pub drawee: BillIdentParticipantWeb,
    pub drawer: BillIdentParticipantWeb,
    pub payee: BillParticipantWeb,
    pub endorsee: Option<BillParticipantWeb>,
    pub endorsements_count: u64,
    pub all_participant_node_ids: Vec<String>,
}

impl From<BillParticipants> for BillParticipantsWeb {
    fn from(val: BillParticipants) -> Self {
        BillParticipantsWeb {
            drawee: val.drawee.into(),
            drawer: val.drawer.into(),
            payee: val.payee.into(),
            endorsee: val.endorsee.map(|e| e.into()),
            endorsements_count: val.endorsements_count,
            all_participant_node_ids: val.all_participant_node_ids,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub enum BillParticipantWeb {
    Anon(BillAnonParticipantWeb),
    Ident(BillIdentParticipantWeb),
}

impl From<BillParticipant> for BillParticipantWeb {
    fn from(val: BillParticipant) -> Self {
        match val {
            BillParticipant::Ident(data) => BillParticipantWeb::Ident(data.into()),
            BillParticipant::Anon(data) => BillParticipantWeb::Anon(data.into()),
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct BillAnonParticipantWeb {
    pub node_id: String,
    pub email: Option<String>,
    pub nostr_relays: Vec<String>,
}

impl From<BillAnonParticipant> for BillAnonParticipantWeb {
    fn from(val: BillAnonParticipant) -> Self {
        BillAnonParticipantWeb {
            node_id: val.node_id,
            email: val.email,
            nostr_relays: val.nostr_relays,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BillIdentParticipantWeb {
    #[serde(rename = "type")]
    pub t: ContactTypeWeb,
    pub node_id: String,
    pub name: String,
    #[serde(flatten)]
    pub postal_address: PostalAddressWeb,
    pub email: Option<String>,
    pub nostr_relays: Vec<String>,
}

impl From<BillIdentParticipant> for BillIdentParticipantWeb {
    fn from(val: BillIdentParticipant) -> Self {
        BillIdentParticipantWeb {
            t: val.t.into(),
            name: val.name,
            node_id: val.node_id,
            postal_address: val.postal_address.into(),
            email: val.email,
            nostr_relays: val.nostr_relays,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationWeb {
    pub id: String,
    pub node_id: Option<String>,
    pub notification_type: NotificationTypeWeb,
    pub reference_id: Option<String>,
    pub description: String,
    pub datetime: DateTimeUtc,
    pub active: bool,
    pub payload: Option<serde_json::Value>,
}

impl From<Notification> for NotificationWeb {
    fn from(val: Notification) -> Self {
        NotificationWeb {
            id: val.id,
            node_id: val.node_id,
            notification_type: val.notification_type.into(),
            reference_id: val.reference_id,
            description: val.description,
            datetime: val.datetime,
            active: val.active,
            payload: val.payload,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NotificationTypeWeb {
    General,
    Bill,
}

impl From<NotificationType> for NotificationTypeWeb {
    fn from(val: NotificationType) -> Self {
        match val {
            NotificationType::Bill => NotificationTypeWeb::Bill,
            NotificationType::General => NotificationTypeWeb::General,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestToPayBitcreditBillPayload {
    pub bill_id: String,
    pub currency: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BillCombinedBitcoinKeyWeb {
    pub private_key: String,
}

impl From<BillCombinedBitcoinKey> for BillCombinedBitcoinKeyWeb {
    fn from(val: BillCombinedBitcoinKey) -> Self {
        BillCombinedBitcoinKeyWeb {
            private_key: val.private_key,
        }
    }
}
