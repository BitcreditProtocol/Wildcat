// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use clwdr_client::ClowderRestClient;
// ----- local imports
use crate::{
    debit::service::ClowderService,
    error::{Error, Result},
};

// ----- end imports

pub struct ClowderCl(pub ClowderRestClient);

#[async_trait]
impl ClowderService for ClowderCl {
    async fn get_sweep(&self, qid: uuid::Uuid, kid: cashu::Id) -> Result<bitcoin::Address> {
        let response = self
            .0
            .request_mint_address(qid, kid)
            .await
            .map_err(Error::ClowderClient)?;
        Ok(response.address.assume_checked())
    }
}
