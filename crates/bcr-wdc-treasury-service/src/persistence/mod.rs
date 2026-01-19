// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
// ----- local imports
use crate::{
    debit::MintQuote,
    debit::{ClowderMintQuoteOnchain, OnchainMeltQuote},
    error::Result,
};
// ----- local modules
pub mod inmemory;
pub mod surreal;

// ----- end imports

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository: Send + Sync {
    async fn store_quote(&self, quote: MintQuote) -> Result<()>;
    async fn delete_quote(&self, qid: String) -> Result<()>;
    async fn list_quotes(&self) -> Result<Vec<MintQuote>>;
    async fn store_onchain_melt(&self, quote_id: uuid::Uuid, data: OnchainMeltQuote) -> Result<()>;
    async fn load_onchain_melt(&self, quote_id: uuid::Uuid) -> Result<OnchainMeltQuote>;
    async fn store_onchain_mint(
        &self,
        quote_id: uuid::Uuid,
        data: ClowderMintQuoteOnchain,
    ) -> Result<()>;
    async fn load_onchain_mint(&self, quote_id: uuid::Uuid) -> Result<ClowderMintQuoteOnchain>;
}
