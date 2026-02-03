// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::cdk_common::payment::PaymentIdentifier;
// ----- local imports
use crate::{error::Result, payment};
// ----- local modules
pub mod inmemory;
pub mod surreal;

// ----- end imports

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait PaymentRepository: Send + Sync {
    async fn load_incoming(&self, reqid: &PaymentIdentifier) -> Result<payment::IncomingRequest>;
    async fn store_incoming(&self, req: payment::IncomingRequest) -> Result<()>;
    async fn update_incoming(&self, req: payment::IncomingRequest) -> Result<()>;
    async fn list_unpaid_incoming_requests(&self) -> Result<Vec<payment::IncomingRequest>>;

    async fn load_outgoing(&self, reqid: &PaymentIdentifier) -> Result<payment::OutgoingRequest>;
    async fn store_outgoing(&self, req: payment::OutgoingRequest) -> Result<()>;
    async fn update_outgoing(&self, req: payment::OutgoingRequest) -> Result<()>;

    async fn store_foreign(&self, new: payment::ForeignPayment) -> Result<()>;
    async fn check_foreign_nonce(&self, nonce: &str) -> Result<Option<payment::ForeignPayment>>;
    async fn check_foreign_reqid(
        &self,
        reqid: &PaymentIdentifier,
    ) -> Result<Option<payment::ForeignPayment>>;
}
