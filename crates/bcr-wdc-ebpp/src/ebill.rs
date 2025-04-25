// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bdk_wallet::bitcoin::Amount;
// ----- local imports
use crate::error::Result;
use crate::service::EBillNode;

// ----- end imports

pub struct DummyEbillNode;

#[async_trait]
impl EBillNode for DummyEbillNode {
    async fn request_to_pay(&self, bill: &str, amount: Amount) -> Result<String> {
        log::info!(
            "DummyEbillNode: request_to_pay called with bill: {}, amount: {}",
            bill,
            amount
        );
        Ok(String::from(
            "wpkh(Ky1BY5QkB6xb3iQQjJQmVcvqc6mkLBaZTW1xCWpf91aFGBh1kyQ7)",
        ))
    }
}
