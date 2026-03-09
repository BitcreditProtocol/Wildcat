// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, wire::clowder::messages as clowder_messages};
use clwdr_client::{ClowderNatsClient, ClowderRestClient};
// ----- local imports
use crate::{
    debit::ClowderClient,
    error::{Error, Result},
};

// ----- end imports

pub struct ClowderCl {
    pub rest: Arc<ClowderRestClient>,
    pub nats: Option<Arc<ClowderNatsClient>>,
}

#[async_trait]
impl ClowderClient for ClowderCl {
    async fn get_sweep(&self, qid: uuid::Uuid) -> Result<bitcoin::Address> {
        let dummy_kid = cashu::Id::from_bytes(&[0_u8; 8])
            .map_err(|_| crate::error::Error::InvalidInput(String::from("Invalid keyset ID")))?;
        let response = self
            .rest
            .request_mint_address(qid, dummy_kid)
            .await
            .map_err(Error::ClowderClient)?;
        Ok(response.address.assume_checked())
    }

    async fn pay_bill(
        &self,
        req: clowder_messages::BillPaymentRequest,
        resp: clowder_messages::BillPaymentResponse,
    ) -> Result<()> {
        let Some(nats) = &self.nats else {
            return Ok(());
        };
        nats.pay_bill(req, resp)
            .await
            .map(|_| ())
            .map_err(Error::ClowderClient)
    }
}
