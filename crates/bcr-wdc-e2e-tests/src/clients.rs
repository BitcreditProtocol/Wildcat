// ----- standard library imports
use std::marker::PhantomData;
// ----- extra library imports
use anyhow::Result;
use bcr_wdc_webapi::keys::ActivateKeysetRequest;
use bcr_wdc_webapi::quotes::{
    EnquireReply, EnquireRequest, ListReplyLight, StatusReply, UpdateQuoteRequest,
    UpdateQuoteResponse,
};
use cashu::nuts::nut02 as cdk02;
use cashu::{MintBolt11Request, MintBolt11Response};
use reqwest::Client as HttpClient;
use reqwest::Url;
use serde::{de::DeserializeOwned, Serialize};
use uuid::Uuid;
// ----- local modules
// ----- end imports

#[derive(Clone)]
pub struct RestClient {
    http: HttpClient,
    token: Option<String>,
}

impl RestClient {
    /// Create a new client with no token yet.
    pub fn new() -> Self {
        let http = HttpClient::builder().build().unwrap();
        RestClient { http, token: None }
    }

    /// Authenticate against an OAuth2 token endpoint using ROPC
    /// and store the access_token for future requests.
    ///
    /// # Parameters
    /// - `token_url`: e.g. `http://localhost:8080/realms/dev/protocol/openid-connect/token`
    /// - `client_id` / `client_secret`: your OAuth2 client credentials
    /// - `username` / `password`: the resource owner credentials
    pub async fn authenticate(
        &mut self,
        token_url: Url,
        client_id: &str,
        client_secret: &str,
        username: &str,
        password: &str,
    ) -> Result<()> {
        #[derive(serde::Deserialize)]
        struct TokenResponse {
            access_token: String,
        }

        let resp: TokenResponse = self
            .http
            .post(token_url)
            .form(&[
                ("grant_type", "password"),
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("username", username),
                ("password", password),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        self.token = Some(resp.access_token);
        Ok(())
    }

    fn authorize(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref tok) = self.token {
            req.bearer_auth(tok)
        } else {
            req
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, url: Url) -> Result<T> {
        let req = self.http.get(url);
        let req = self.authorize(req);
        let resp = req.send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn post<Req: Serialize, Res: DeserializeOwned>(
        &self,
        url: Url,
        body: &Req,
    ) -> Result<Res> {
        let req = self.http.post(url).json(body);
        let req = self.authorize(req);
        let resp = req.send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn post_<Req: Serialize>(&self, url: Url, body: &Req) -> Result<()> {
        let req = self.http.post(url).json(body);
        let req = self.authorize(req);
        req.send().await?.error_for_status()?;
        Ok(())
    }
}

impl Default for RestClient {
    fn default() -> Self {
        Self::new()
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
    pub async fn admin_credit_quote_list(&self) -> Result<ListReplyLight> {
        let url = self.url("v1/admin/credit/quote");
        self.client.get(url).await
    }
    pub async fn authenticate(
        &mut self,
        token_url: Url,
        client_id: &str,
        client_secret: &str,
        username: &str,
        password: &str,
    ) -> Result<()> {
        self.client
            .authenticate(token_url, client_id, client_secret, username, password)
            .await
    }
}
