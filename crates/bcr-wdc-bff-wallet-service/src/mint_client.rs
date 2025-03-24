// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use cashu::mint_url::MintUrl;
use cashu::nuts::nut01 as cdk01;
use cashu::nuts::nut06 as cdk06;
use cdk::HttpClient;
use cdk::wallet::client::MintConnector;
// ----- local imports
use crate::error::{Error, Result};
use crate::service::MintService;

#[derive(Debug, Clone)]
pub struct MintClient(HttpClient);

impl MintClient {
    pub async fn new(mint_url: MintUrl) -> Result<Self> {
        let cl = HttpClient::new(mint_url);
        Ok(Self(cl))
    }
}

#[async_trait]
impl MintService for MintClient {
    async fn info(&self) -> Result<cdk06::MintInfo> {
        let response = self.0.get_mint_info().await;
        match response {
            Ok(info) => Ok(info),
            Err(e) => Err(Error::CDKClient(e)),
        }
    }
    async fn keys(&self) -> Result<cdk01::KeysResponse> {
        let response = self.0.get_mint_keys().await;
        match response {
            Ok(keyset) => Ok(cdk01::KeysResponse { keysets: keyset }),
            Err(e) => Err(Error::CDKClient(e)),
        }
    }
}
