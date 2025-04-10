use async_trait::async_trait;
use bcr_wdc_key_client::KeyClient;
use cashu::{
    CheckStateRequest, CheckStateResponse, Id, KeySet, KeysetResponse, MeltBolt11Request,
    MeltQuoteBolt11Request, MeltQuoteBolt11Response, MintBolt11Request, MintBolt11Response,
    MintInfo, MintQuoteBolt11Request, MintQuoteBolt11Response, RestoreRequest, RestoreResponse,
    SwapRequest, SwapResponse,
};
use cdk::Error;
use cdk::wallet::MintConnector;
use std::fmt::Debug;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Service {
    pub mint_service: Arc<dyn MintConnector + Send + Sync>,
    pub key_service: KeyClient,
}

#[async_trait]
impl MintConnector for Service {
    async fn get_mint_keys(&self) -> Result<Vec<KeySet>, Error> {
        self.mint_service.get_mint_keys().await
        // TODO: merge with key service response
        // let key_keys = self.key_service.keys().await;
    }

    async fn get_mint_keyset(&self, keyset_id: Id) -> Result<KeySet, Error> {
        let key_response = self.key_service.keys(keyset_id).await;
        match key_response {
            Ok(it) => Ok(it),
            Err(_) => self.mint_service.get_mint_keyset(keyset_id).await,
        }
    }

    async fn get_mint_keysets(&self) -> Result<KeysetResponse, Error> {
        self.mint_service.get_mint_keysets().await
    }

    async fn post_mint_quote(
        &self,
        request: MintQuoteBolt11Request,
    ) -> Result<MintQuoteBolt11Response<String>, Error> {
        self.mint_service.post_mint_quote(request).await
    }

    async fn get_mint_quote_status(
        &self,
        quote_id: &str,
    ) -> Result<MintQuoteBolt11Response<String>, Error> {
        self.mint_service.get_mint_quote_status(quote_id).await
    }

    async fn post_mint(
        &self,
        request: MintBolt11Request<String>,
    ) -> Result<MintBolt11Response, Error> {
        self.mint_service.post_mint(request).await
    }

    async fn post_melt_quote(
        &self,
        request: MeltQuoteBolt11Request,
    ) -> Result<MeltQuoteBolt11Response<String>, Error> {
        self.mint_service.post_melt_quote(request).await
    }

    async fn get_melt_quote_status(
        &self,
        quote_id: &str,
    ) -> Result<MeltQuoteBolt11Response<String>, Error> {
        self.mint_service.get_melt_quote_status(quote_id).await
    }

    async fn post_melt(
        &self,
        request: MeltBolt11Request<String>,
    ) -> Result<MeltQuoteBolt11Response<String>, Error> {
        self.mint_service.post_melt(request).await
    }

    async fn post_swap(&self, request: SwapRequest) -> Result<SwapResponse, Error> {
        self.mint_service.post_swap(request).await
    }

    async fn get_mint_info(&self) -> Result<MintInfo, Error> {
        self.mint_service.get_mint_info().await
    }

    async fn post_check_state(
        &self,
        request: CheckStateRequest,
    ) -> Result<CheckStateResponse, Error> {
        self.mint_service.post_check_state(request).await
    }

    async fn post_restore(&self, request: RestoreRequest) -> Result<RestoreResponse, Error> {
        self.mint_service.post_restore(request).await
    }

    async fn get_auth_wallet(&self) -> Option<cdk::wallet::AuthWallet> {
        self.mint_service.get_auth_wallet().await
    }

    async fn set_auth_wallet(&self, wallet: Option<cdk::wallet::AuthWallet>) {
        self.mint_service.set_auth_wallet(wallet).await
    }
}
