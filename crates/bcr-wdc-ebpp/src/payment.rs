// ----- standard library imports
// ----- extra library imports
use bdk_wallet::bitcoin as btc;
use cashu::{MeltQuoteState, MintQuoteState};
use cdk_common::payment::PaymentIdentifier;
use uuid::Uuid;
// ----- local imports

// ----- end imports

#[derive(Debug, Clone)]
pub struct IncomingRequest {
    pub reqid: PaymentIdentifier,
    pub payment_type: PaymentType,
    pub amount: btc::Amount,
    pub status: MintQuoteState,
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone)]
pub enum PaymentType {
    EBill {
        recipient: btc::Address,
        sweep: btc::Address,
    },
    ClowderOnchain(Uuid),
}
impl PaymentType {
    pub fn recipient(&self) -> Option<btc::Address> {
        match self {
            PaymentType::EBill { recipient, .. } => Some(recipient.clone()),
            PaymentType::ClowderOnchain(_) => None,
        }
    }

    pub fn clowder_quote(&self) -> Option<Uuid> {
        match self {
            PaymentType::ClowderOnchain(uuid) => Some(*uuid),
            _ => None,
        }
    }
}

impl std::fmt::Display for IncomingRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        if let Some(address) = self.payment_type.recipient() {
            let mut uri: bip21::Uri = bip21::Uri::new(address);
            uri.amount = Some(self.amount);
            write!(f, "{uri}")
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutgoingRequest {
    pub reqid: PaymentIdentifier,
    pub amount: cashu::Amount,
    pub state: MeltQuoteState,
}
impl OutgoingRequest {
    pub fn new(reqid: PaymentIdentifier, amount: cashu::Amount) -> Self {
        Self {
            reqid,
            amount,
            state: MeltQuoteState::Pending,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ForeignPayment {
    pub reqid: PaymentIdentifier,
    pub nonce: String,
    pub amount: btc::Amount,
}
