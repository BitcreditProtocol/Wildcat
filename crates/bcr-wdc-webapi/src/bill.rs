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
    pub current_waiting_state: Option<wire_bill::BillCurrentWaitingState>,
}

impl TryFrom<bill::BitcreditBillResult> for BitcreditBill {
    type Error = convert::Error;
    fn try_from(val: bill::BitcreditBillResult) -> std::result::Result<Self, Self::Error> {
        let retv = Self {
            id: val.id,
            participants: convert::billparticipants_ebill2wire(val.participants),
            data: convert::billdata_ebill2wire(val.data)?,
            status: convert::billstatus_ebill2wire(val.status),
            current_waiting_state: val
                .current_waiting_state
                .map(convert::billcurrentwaitingstate_ebill2wire),
        };
        Ok(retv)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestToPayBitcreditBillPayload {
    pub bill_id: BillId,
    pub currency: String,
}
