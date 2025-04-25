// ----- standard library imports
// ----- extra library imports
use bdk_wallet::bitcoin as btc;
use cashu::{MeltQuoteState, MintQuoteState};
use uuid::Uuid;
// ----- local imports
use crate::error::{Error, Result};

// ----- end imports

#[derive(Debug, Clone)]
pub struct IncomingRequest {
    pub reqid: Uuid,
    pub payment_type: PaymentType,
    pub amount: btc::Amount,
    pub status: MintQuoteState,
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone)]
pub enum PaymentType {
    OnChain(btc::Address),
    EBill(btc::Address),
}
impl PaymentType {
    pub fn recipient(&self) -> btc::Address {
        match self {
            PaymentType::EBill(add) => add.clone(),
            PaymentType::OnChain(add) => add.clone(),
        }
    }
}

impl std::fmt::Display for IncomingRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        let uri = match &self.payment_type {
            PaymentType::OnChain(address) => {
                let mut uri: bip21::Uri = bip21::Uri::new(address.clone());
                uri.amount = Some(self.amount);
                uri
            }
            PaymentType::EBill(address) => {
                let mut uri: bip21::Uri = bip21::Uri::new(address.clone());
                uri.amount = Some(self.amount);
                uri
            }
        };
        write!(f, "{}", uri)
    }
}

#[derive(Debug, Clone)]
pub struct OutgoingRequest {
    pub reqid: Uuid,
    pub recipient: btc::Address,
    pub amount: btc::Amount,
    pub status: MeltQuoteState,
    pub proof: Option<btc::Txid>,
    pub total_spent: Option<btc::Amount>,
}

impl OutgoingRequest {
    pub fn new(reqid: Uuid, uri: bip21::Uri) -> Result<Self> {
        let amount = uri.amount.ok_or(Error::UnknownAmount)?;
        Ok(Self {
            reqid,
            recipient: uri.address,
            amount,
            status: MeltQuoteState::Unpaid,
            proof: None,
            total_spent: None,
        })
    }
}
