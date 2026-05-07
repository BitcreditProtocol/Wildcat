// ----- standard library imports
use std::marker::PhantomData;
// ----- extra library imports
use anyhow::Result;
use bcr_common::{
    cashu,
    client::{
        core::Client as CoreClient, ebill::Client as EbillClient, mint::Client as MintClient,
        quote::Client as QuoteClient,
    },
    wire::{identity as wire_identity, quotes as wire_quotes, swap as wire_swap},
};
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
    core_cl: CoreClient,
    quote_cl: QuoteClient,
    mint_cl: MintClient,
    client: RestClient,
    _marker: PhantomData<T>,
}

impl<T> Service<T> {
    pub fn new(base_url: String) -> Self {
        let url = reqwest::Url::parse(&base_url).unwrap();
        Self {
            ebill_cl: EbillClient::new(url.clone()),
            core_cl: CoreClient::new(url.clone()),
            quote_cl: QuoteClient::new(url.clone()),
            mint_cl: MintClient::new(url),
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
        bill: wire_quotes::SharedBill,
        minting_pubkey: cashu::PublicKey,
        signing_key: &bitcoin::key::Keypair,
    ) -> Uuid {
        self.mint_cl
            .enquire(bill, minting_pubkey, signing_key)
            .await
            .unwrap()
    }

    pub async fn lookup_credit_quote(&self, quote_id: Uuid) -> wire_quotes::StatusReply {
        self.mint_cl.lookup(quote_id).await.unwrap()
    }

    pub async fn list_keysets(&self) -> Vec<cashu::KeySetInfo> {
        self.core_cl
            .list_keyset_info(Default::default())
            .await
            .unwrap()
    }

    pub async fn list_keys(&self, kid: cashu::Id) -> cashu::KeySet {
        self.core_cl.keys(kid).await.unwrap()
    }

    pub async fn accept_quote(&self, qid: Uuid) {
        self.mint_cl.accept_offer(qid).await.unwrap();
    }

    pub async fn mint_ebill(
        &self,
        qid: Uuid,
        outputs: Vec<cashu::BlindedMessage>,
        sk: cashu::SecretKey,
    ) -> Vec<cashu::BlindSignature> {
        self.mint_cl.ebill_mint(qid, outputs, sk).await.unwrap()
    }
    /// GET v1/info
    pub async fn mint_info(&self) -> cashu::nut06::MintInfo {
        let url = self.url("v1/info");
        self.client.get(url).await.unwrap()
    }

    /// GET v1/wildcat — used to discover the local Alpha's `clowder_node_id`.
    pub async fn wildcat_info(&self) -> bcr_common::wire::info::WildcatInfo {
        let url = self.url("v1/wildcat");
        self.client.get(url).await.unwrap()
    }

    pub async fn commit_swap(
        &self,
        request: wire_swap::SwapCommitmentRequest,
    ) -> wire_swap::SwapCommitmentResponse {
        let url = self.url("v1/swap/commitment");
        let resp = self
            .client
            .http
            .post(url)
            .json(&request)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        resp.json().await.unwrap()
    }

    pub async fn swap(
        &self,
        inputs: Vec<cashu::Proof>,
        outputs: Vec<cashu::BlindedMessage>,
        commitment: bitcoin::secp256k1::schnorr::Signature,
        attestation: bcr_common::wire::attestation::IssuanceAttestation,
    ) -> Vec<cashu::BlindSignature> {
        self.mint_cl
            .swap(inputs, outputs, commitment, attestation)
            .await
            .unwrap()
    }

    /// Acquire a Beta-issued attestation for the given inputs by hitting
    /// `POST /v1/attest/issuance` (Envoy-routed to the local Clowder node).
    pub async fn acquire_attestation(
        &self,
        alpha_id: bitcoin::secp256k1::PublicKey,
        proofs: &[cashu::Proof],
    ) -> bcr_common::wire::attestation::IssuanceAttestation {
        use bcr_common::wire::{attestation as wire_att, keys as wire_keys};
        let inputs: Vec<wire_keys::ProofFingerprint> = proofs
            .iter()
            .map(|p| wire_keys::ProofFingerprint::try_from(p.clone()).unwrap())
            .collect();
        let request = wire_att::IssuanceAttestationRequest { alpha_id, inputs };
        let url = self.url("v1/attest/issuance");
        self.client
            .http
            .post(url)
            .json(&request)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}

impl Service<AdminService> {
    pub async fn offer_quote(
        &self,
        quote_id: Uuid,
        discounted: bitcoin::Amount,
    ) -> wire_quotes::UpdateQuoteResponse {
        self.quote_cl
            .offer(quote_id, discounted, None)
            .await
            .unwrap()
    }

    pub async fn admin_credit_quote_list(&self) -> Result<wire_quotes::ListReplyLight> {
        self.quote_cl
            .list(wire_quotes::ListParam::default())
            .await
            .map_err(Into::into)
    }
    pub async fn admin_ebill_identity_details(&self) -> Result<wire_identity::Identity> {
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
        self.client
            .authenticate(token_url, client_id, client_secret, username, password)
            .await
    }
}
