// ----- standard library imports
use std::marker::PhantomData;
// ----- extra library imports
use anyhow::Result;
use bcr_wdc_webapi::keys::ActivateKeysetRequest;
use bcr_wdc_webapi::quotes::{
    EnquireReply, EnquireRequest, StatusReply, UpdateQuoteRequest, UpdateQuoteResponse,
};
use cashu::nuts::nut02 as cdk02;
use cashu::{MintBolt11Request, MintBolt11Response};
use reqwest::Client as HttpClient;
use reqwest::Url;
use serde::{de::DeserializeOwned, Serialize};
use uuid::Uuid;
// ----- local modules
// ----- end imports

pub struct RestClient {
    http: HttpClient,
}

impl RestClient {
    pub fn new() -> Self {
        let http = HttpClient::builder().build().unwrap();
        RestClient { http }
    }

    pub async fn get<T: DeserializeOwned>(&self, url: Url) -> Result<T> {
        let resp = self.http.get(url).send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn post<Req: Serialize, Res: DeserializeOwned>(
        &self,
        url: Url,
        body: &Req,
    ) -> Result<Res> {
        let resp = self
            .http
            .post(url)
            .json(body)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn post_<Req: Serialize>(&self, url: Url, body: &Req) -> Result<()> {
        self.http
            .post(url)
            .json(body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

pub struct Service<T> {
    base_url: String,
    client: RestClient,
    _marker: PhantomData<T>,
}

impl<T> Service<T> {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: RestClient::new(),
            _marker: PhantomData,
        }
    }
    fn url(&self, path: &str) -> Url {
        Url::parse(&format!("{}/{}", self.base_url, path)).unwrap()
    }
}

pub struct UserService {}
pub struct AdminService {}

impl Service<UserService> {
    pub async fn mint_credit_quote(&self, req: EnquireRequest) -> EnquireReply {
        let url = self.url("v1/mint/credit/quote");
        self.client.post(url, &req).await.unwrap()
    }
    pub async fn lookup_credit_quote(&self, quote_id: Uuid) -> StatusReply {
        // GET Uuid, StatusReply
        let url = self.url(&format!("v1/mint/credit/quote/{quote_id}"));
        self.client.get(url).await.unwrap()
    }
    pub async fn list_keysets(&self) -> cdk02::KeysetResponse {
        let url = self.url("v1/keysets");
        self.client.get(url).await.unwrap()
    }
    pub async fn mint_ebill(&self, req: MintBolt11Request<Uuid>) -> MintBolt11Response {
        let url = self.url("v1/mint/ebill");
        self.client.post(url, &req).await.unwrap()
    }
}

impl Service<AdminService> {
    pub async fn keys_activate(&self, req: ActivateKeysetRequest) {
        let url = self.url("v1/admin/keys/activate");
        self.client.post_(url, &req).await.unwrap()
    }
    pub async fn admin_credit_quote(
        &self,
        quote_id: Uuid,
        quote_req: UpdateQuoteRequest,
    ) -> UpdateQuoteResponse {
        let url = self.url(&format!("v1/admin/credit/quote/{quote_id}"));
        self.client.post(url, &quote_req).await.unwrap()
    }
}
