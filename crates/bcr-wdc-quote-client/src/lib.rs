// ----- standard library imports
// ----- extra library imports
use bcr_wdc_webapi::quotes as web_quotes;
pub use reqwest::Url;
use thiserror::Error;
use uuid::Uuid;
// ----- local imports

// ----- end imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("resource not found {0}")]
    ResourceNotFound(Uuid),
    #[error("invalid request")]
    InvalidRequest,
    #[error("Schnorr signing error")]
    Signing(#[from] bcr_wdc_utils::keys::SchnorrBorshMsgError),

    #[error("internal error {0}")]
    Reqwest(#[from] reqwest::Error),
}

#[derive(Debug, Clone)]
pub struct QuoteClient {
    cl: reqwest::Client,
    base: reqwest::Url,
}

impl QuoteClient {
    pub fn new(base: reqwest::Url) -> Self {
        Self {
            cl: reqwest::Client::new(),
            base,
        }
    }

    pub async fn enquire(
        &self,
        bill: web_quotes::BillInfo,
        mint_pubkey: cashu::PublicKey,
        signing_key: &bitcoin::secp256k1::Keypair,
    ) -> Result<Uuid> {
        let request = bcr_wdc_webapi::quotes::EnquireRequest {
            content: bill,
            public_key: mint_pubkey,
        };
        let signature = bcr_wdc_utils::keys::schnorr_sign_borsh_msg_with_key(&request, signing_key)
            .map_err(Error::Signing)?;

        let signed = web_quotes::SignedEnquireRequest { request, signature };

        let url = self
            .base
            .join("/v1/mint/credit/quote")
            .expect("enquire relative path");
        let res = self.cl.post(url).json(&signed).send().await?;
        let reply = res.json::<web_quotes::EnquireReply>().await?;
        Ok(reply.id)
    }

    pub async fn lookup(&self, qid: Uuid) -> Result<web_quotes::StatusReply> {
        let url = self
            .base
            .join(&format!("/v1/mint/credit/quote/{qid}"))
            .expect("lookup relative path");
        let res = self.cl.get(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(qid));
        }
        let reply = res.json::<web_quotes::StatusReply>().await?;
        Ok(reply)
    }

    pub async fn list(&self, params: web_quotes::ListParam) -> Result<web_quotes::ListReplyLight> {
        let url = self
            .base
            .join("/v1/admin/credit/quote")
            .expect("list relative path");
        let mut request = self.cl.get(url);
        let web_quotes::ListParam {
            bill_maturity_date_from,
            bill_maturity_date_to,
            status,
            bill_id,
            bill_drawee_id,
            bill_drawer_id,
            bill_payer_id,
            bill_holder_id,
            ..
        } = params;
        if let Some(date) = bill_maturity_date_from {
            request = request.query(&[("bill_maturity_date_from", date.to_string())]);
        }
        if let Some(date) = bill_maturity_date_to {
            request = request.query(&[("bill_maturity_date_to", date.to_string())]);
        }
        if let Some(status) = status {
            request = request.query(&[("status", status.to_string())]);
        }
        if let Some(bill_id) = bill_id {
            request = request.query(&[("bill_id", bill_id)]);
        }
        if let Some(bill_drawee_id) = bill_drawee_id {
            request = request.query(&[("bill_drawee_id", bill_drawee_id)]);
        }
        if let Some(bill_drawer_id) = bill_drawer_id {
            request = request.query(&[("bill_drawer_id", bill_drawer_id)]);
        }
        if let Some(bill_payer_id) = bill_payer_id {
            request = request.query(&[("bill_payer_id", bill_payer_id)]);
        }
        if let Some(bill_holder_id) = bill_holder_id {
            request = request.query(&[("bill_holder_id", bill_holder_id)]);
        }
        let response = request.send().await?;

        let reply = response.json::<web_quotes::ListReplyLight>().await?;
        Ok(reply)
    }

    pub async fn resolve(&self, qid: Uuid, action: web_quotes::ResolveOffer) -> Result<()> {
        let url = self
            .base
            .join(&format!("/v1/mint/credit/quote/{qid}"))
            .expect("resolve relative path");
        let res = self.cl.post(url).json(&action).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(qid));
        }
        Ok(())
    }

    pub async fn cancel(&self, qid: Uuid) -> Result<()> {
        let url = self
            .base
            .join(&format!("/v1/mint/credit/quote/{qid}"))
            .expect("resolve relative path");
        let res = self.cl.delete(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(qid));
        }
        Ok(())
    }
}
