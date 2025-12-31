// ----- standard library imports
// ----- extra library imports
use bdk_wallet::bitcoin as btc;
use cashu::{MeltQuoteState, MintQuoteState};
use cdk_common::payment::PaymentIdentifier;
use uuid::Uuid;
// ----- local imports
use crate::error::{Error, Result};

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
    OnChain(btc::Address),
    EBill(btc::Address),
    ClowderOnchain(Uuid),
}
impl PaymentType {
    pub fn recipient(&self) -> Option<btc::Address> {
        match self {
            PaymentType::EBill(add) => Some(add.clone()),
            PaymentType::OnChain(add) => Some(add.clone()),
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

#[derive(Debug, Clone)]
pub struct OutgoingRequest {
    pub reqid: PaymentIdentifier,
    pub recipient: btc::Address,
    pub amount: btc::Amount,
    pub reserved_fees: btc::Amount,
    pub status: MeltQuoteState,
    pub proof: Option<btc::Txid>,
    pub total_spent: Option<btc::Amount>,
}

impl OutgoingRequest {
    pub fn new(
        reqid: PaymentIdentifier,
        uri: bip21::Uri,
        reserved_fees: btc::Amount,
    ) -> Result<Self> {
        let amount = uri.amount.ok_or(Error::UnknownAmount)?;
        Ok(Self {
            reqid,
            recipient: uri.address,
            amount,
            reserved_fees,
            status: MeltQuoteState::Unpaid,
            proof: None,
            total_spent: None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ForeignPayment {
    pub reqid: PaymentIdentifier,
    pub nonce: String,
    pub amount: btc::Amount,
}
