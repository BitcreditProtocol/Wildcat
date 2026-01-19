// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::wire::clowder::messages as clowder_messages;
use clwdr_client::{ClowderNatsClient, ClowderRestClient};
// ----- local imports
use crate::{
    debit::service::{ClowderReadService, ClowderWriteService},
    error::{Error, Result},
};

// ----- end imports

pub struct ClowderCl(pub ClowderRestClient);

#[async_trait]
impl ClowderReadService for ClowderCl {
    async fn get_sweep(&self, qid: uuid::Uuid, kid: cashu::Id) -> Result<bitcoin::Address> {
        let response = self
            .0
            .request_mint_address(qid, kid)
            .await
            .map_err(Error::ClowderClient)?;
        Ok(response.address.assume_checked())
    }
}

pub struct ClowderNatsCl(pub std::sync::Arc<ClowderNatsClient>);

#[async_trait]
impl ClowderWriteService for ClowderNatsCl {
    async fn pay_bill(
        &self,
        req: clowder_messages::BillPaymentRequest,
        resp: clowder_messages::BillPaymentResponse,
    ) -> Result<()> {
        self.0
            .pay_bill(req, resp)
            .await
            .map(|_| ())
            .map_err(Error::ClowderClient)
    }
}
