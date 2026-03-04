// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use uuid::Uuid;
// ----- local modules
pub mod inmemory;
pub mod surreal;
// ----- local imports
use crate::{
    error::Result,
    quotes::{LightQuote, Quote, Status},
    service::{ListFilters, SortOrder},
    TStamp,
};

// ----- end imports

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository {
    async fn load(&self, id: uuid::Uuid) -> Result<Option<Quote>>;
    async fn update_status_if_pending(&self, id: uuid::Uuid, quote: Status) -> Result<()>;
    async fn update_status_if_offered(&self, id: uuid::Uuid, quote: Status) -> Result<()>;
    async fn update_status_if_accepted(&self, id: uuid::Uuid, quote: Status) -> Result<()>;
    async fn list_pendings(&self, since: Option<TStamp>) -> Result<Vec<Uuid>>;
    async fn list_light(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
    ) -> Result<Vec<LightQuote>>;
    async fn search_by_bill(&self, bill: &BillId, endorser: &NodeId) -> Result<Vec<Quote>>;
    async fn store(&self, quote: Quote) -> Result<()>;
}
