// ----- standard library imports
// ----- extra library imports
use bdk_wallet::bitcoin;
use cashu::{Amount, CurrencyUnit};
use cdk_common::MintQuoteState;
use uuid::Uuid;
// ----- local imports

// ----- end imports

#[derive(Debug, Clone)]
pub struct Request {
    pub reqid: Uuid,
    pub payment_type: PaymentType,
    pub amount: Amount,
    pub currency: CurrencyUnit,
    pub status: MintQuoteState,
}

#[derive(Debug, Clone)]
pub enum PaymentType {
    OnChain(bitcoin::Address),
    EBill(bitcoin::Address),
}

impl std::fmt::Display for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        let uri = match &self.payment_type {
            PaymentType::OnChain(address) => {
                assert_eq!(self.currency, CurrencyUnit::Sat);
                let amount = bitcoin::Amount::from_sat(*self.amount.as_ref());
                let mut uri: bip21::Uri = bip21::Uri::new(address.clone());
                uri.amount = Some(amount);
                uri
            }
            PaymentType::EBill(address) => {
                let amount = bitcoin::Amount::from_sat(*self.amount.as_ref());
                let mut uri: bip21::Uri = bip21::Uri::new(address.clone());
                uri.amount = Some(amount);
                uri
            }
        };
        write!(f, "{}", uri)
    }
}
