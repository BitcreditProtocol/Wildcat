// ----- standard library imports
use std::marker::PhantomData;
// ----- extra library imports
use anyhow::{anyhow, Result};
use bcr_common::{KeysClient, SwapClient};
use bcr_wdc_ebill_client::EbillClient;
use bcr_wdc_quote_client::QuoteClient;
use bcr_wdc_treasury_client::TreasuryClient;
use bcr_wdc_webapi::{
    identity::Identity,
    quotes::{ListReplyLight, StatusReply, UpdateQuoteResponse},
};
use bcr_wdc_webapi::{quotes as web_quotes, wallet::ECashBalance};
use reqwest::Client as HttpClient;
use reqwest::Url;
use serde::de::DeserializeOwned;
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
}

impl Default for RestClient {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Service<T> {
    base_url: String,
    ebill_cl: EbillClient,
    key_cl: KeysClient,
    quote_cl: QuoteClient,
    swap_cl: SwapClient,
    treasury_cl: TreasuryClient,
    client: RestClient,
    _marker: PhantomData<T>,
}

impl<T> Service<T> {
    pub fn new(base_url: String) -> Self {
        let url = reqwest::Url::parse(&base_url).unwrap();
        Self {
            ebill_cl: EbillClient::new(url.clone()),
            key_cl: KeysClient::new(url.clone()),
            quote_cl: QuoteClient::new(url.clone()),
            swap_cl: SwapClient::new(url.clone()),
            treasury_cl: TreasuryClient::new(url),
            client: RestClient::new(),
            _marker: PhantomData,
            base_url,
        }
    }
    fn url(&self, path: &str) -> Url {
        Url::parse(&format!("{}/{}", self.base_url, path)).unwrap()
    }
}

pub struct UserService {}
pub struct AdminService {}

impl Service<UserService> {
    pub async fn mint_credit_quote(
        &self,
        bill: web_quotes::SharedBill,
        miniting_pubkey: cashu::PublicKey,
        signing_key: &bitcoin::key::Keypair,
    ) -> Uuid {
        self.quote_cl
            .enquire(bill, miniting_pubkey, signing_key)
            .await
            .unwrap()
    }

    pub async fn lookup_credit_quote(&self, quote_id: Uuid) -> StatusReply {
        self.quote_cl.lookup(quote_id).await.unwrap()
    }

    pub async fn list_keysets(&self) -> Vec<cashu::KeySetInfo> {
        self.key_cl.list_keyset_info().await.unwrap()
    }

    pub async fn list_keys(&self, kid: cashu::Id) -> cashu::KeySet {
        self.key_cl.keys(kid).await.unwrap()
    }

    pub async fn accept_quote(&self, qid: Uuid) {
        self.quote_cl.accept_offer(qid).await.unwrap();
    }

    pub async fn mint_ebill(
        &self,
        qid: Uuid,
        outputs: Vec<cashu::BlindedMessage>,
        sk: cashu::SecretKey,
    ) -> Vec<cashu::BlindSignature> {
        self.key_cl.mint(qid, outputs, sk).await.unwrap()
    }
    /// GET v1/info
    pub async fn mint_info(&self) -> cashu::nut06::MintInfo {
        let url = self.url("v1/info");
        self.client.get(url).await.unwrap()
    }

    pub async fn swap(
        &self,
        inputs: Vec<cashu::Proof>,
        outputs: Vec<cashu::BlindedMessage>,
    ) -> Vec<cashu::BlindSignature> {
        self.swap_cl.swap(inputs, outputs).await.unwrap()
    }
}

impl Service<AdminService> {
    pub async fn enable_minting_for_quote_id(&self, qid: Uuid) {
        self.quote_cl.enable_minting(qid).await.unwrap();
    }

    pub async fn offer_quote(
        &self,
        quote_id: Uuid,
        discounted: bitcoin::Amount,
    ) -> UpdateQuoteResponse {
        self.quote_cl
            .offer(quote_id, discounted, None)
            .await
            .unwrap()
    }

    pub async fn admin_credit_quote_list(&self) -> Result<ListReplyLight> {
        self.quote_cl
            .list(web_quotes::ListParam::default())
            .await
            .map_err(Into::into)
    }
    pub async fn admin_balance_credit(&self) -> Result<ECashBalance> {
        self.treasury_cl.crsat_balance().await.map_err(Into::into)
    }
    pub async fn admin_ebill_identity_details(&self) -> Result<Identity> {
        self.ebill_cl.get_identity().await.map_err(Into::into)
    }
    pub async fn authenticate(
        &mut self,
        token_url: Url,
        client_id: &str,
        client_secret: &str,
        username: &str,
        password: &str,
    ) -> Result<()> {
        self.ebill_cl
            .authenticate(
                token_url.clone(),
                client_id,
                client_secret,
                username,
                password,
            )
            .await
            .map_err(|e| anyhow!(e))?;
        self.quote_cl
            .authenticate(
                token_url.clone(),
                client_id,
                client_secret,
                username,
                password,
            )
            .await
            .map_err(|e| anyhow!(e))?;
        self.treasury_cl
            .authenticate(
                token_url.clone(),
                client_id,
                client_secret,
                username,
                password,
            )
            .await
            .map_err(|e| anyhow!(e))?;
        self.client
            .authenticate(token_url, client_id, client_secret, username, password)
            .await
    }
}
