use async_trait::async_trait;
use cashu::{
    CheckStateRequest, CheckStateResponse, Id, KeySet, KeysetResponse, MeltBolt11Request,
    MeltQuoteBolt11Request, MeltQuoteBolt11Response, MintBolt11Request, MintBolt11Response,
    MintInfo, MintQuoteBolt11Request, MintQuoteBolt11Response, RestoreRequest, RestoreResponse,
    SwapRequest, SwapResponse,
};
use std::fmt::{Debug, Formatter};
use std::sync::Arc;
// ----- standard library imports
// ----- extra library imports
use bcr_wdc_key_client::KeyClient;
use cdk::Error;
use cdk::wallet::client::MintConnector;

#[derive(Clone)]
pub struct Service {
    pub mint_service: Arc<dyn MintConnector + Send + Sync>,
    pub key_service: KeyClient,
}

impl Debug for Service {
    fn fmt(&self, _f: &mut Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

#[async_trait]
impl MintConnector for Service {
    /// Get Active Mint Keys [NUT-01]
    async fn get_mint_keys(&self) -> Result<Vec<KeySet>, Error> {
        let response = self.mint_service.get_mint_keys().await;
        // TODO: merge with key service response
        // let key_keys = self.key_service.keys().await;
        match response {
            Ok(it) => Ok(it),
            Err(e) => Err(e),
        }
    }

    async fn get_mint_keyset(&self, keyset_id: Id) -> Result<KeySet, Error> {
        let response = self.mint_service.get_mint_keyset(keyset_id).await;
        match response {
            Ok(it) => Ok(it),
            Err(e) => Err(e),
        }
    }

    async fn get_mint_keysets(&self) -> Result<KeysetResponse, Error> {
        let response = self.mint_service.get_mint_keysets().await;
        match response {
            Ok(it) => Ok(it),
            Err(e) => Err(e),
        }
    }

    async fn post_mint_quote(
        &self,
        request: MintQuoteBolt11Request,
    ) -> Result<MintQuoteBolt11Response<String>, Error> {
        self.mint_service.post_mint_quote(request)
    }

    async fn get_mint_quote_status(
        &self,
        quote_id: &str,
    ) -> Result<MintQuoteBolt11Response<String>, Error> {
        self.mint_service.get_mint_quote_status(quote_id)
    }

    async fn post_mint(
        &self,
        request: MintBolt11Request<String>,
    ) -> Result<MintBolt11Response, Error> {
        self.mint_service.post_mint(request)
    }

    async fn post_melt_quote(
        &self,
        request: MeltQuoteBolt11Request,
    ) -> Result<MeltQuoteBolt11Response<String>, Error> {
        self.mint_service.post_melt_quote(request)
    }

    async fn get_melt_quote_status(
        &self,
        quote_id: &str,
    ) -> Result<MeltQuoteBolt11Response<String>, Error> {
        self.mint_service.get_melt_quote_status(quote_id)
    }

    async fn post_melt(
        &self,
        request: MeltBolt11Request<String>,
    ) -> Result<MeltQuoteBolt11Response<String>, Error> {
        self.mint_service.post_melt(request)
    }

    async fn post_swap(&self, request: SwapRequest) -> Result<SwapResponse, Error> {
        self.mint_service.post_swap(request)
    }

    async fn get_mint_info(&self) -> Result<MintInfo, Error> {
        self.mint_service.get_mint_info()
    }

    async fn post_check_state(
        &self,
        request: CheckStateRequest,
    ) -> Result<CheckStateResponse, Error> {
        self.mint_service.post_check_state(request)
    }

    async fn post_restore(&self, request: RestoreRequest) -> Result<RestoreResponse, Error> {
        self.mint_service.post_restore(request)
    }
}
