// ----- standard library imports
// ----- extra library imports
use bcr_wdc_webapi::keys as web_keys;
use cashu::nut00 as cdk00;
use cashu::nut02 as cdk02;
use thiserror::Error;
// ----- local modules
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("URL parse error {0}")]
    Url(#[from] url::ParseError),
    #[error("resource not found {0}")]
    ResourceNotFound(cdk02::Id),
    #[error("invalid request")]
    InvalidRequest,

    #[error("internal error {0}")]
    Reqwest(#[from] reqwest::Error),
}

#[derive(Debug, Clone)]
pub struct KeyClient {
    cl: reqwest::Client,
    base: reqwest::Url,
}

impl KeyClient {
    pub fn new(base: &str) -> Result<Self> {
        let url = reqwest::Url::parse(base)?;
        Ok(Self {
            cl: reqwest::Client::new(),
            base: url,
        })
    }

    pub async fn keys(&self, kid: cdk02::Id) -> Result<cdk02::KeySet> {
        let url = self.base.join(&format!("/v1/keys/{}", kid))?;
        let res = self.cl.get(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(kid));
        }
        let ks = res.json::<cdk02::KeySet>().await?;
        Ok(ks)
    }

    pub async fn keyset(&self, kid: cdk02::Id) -> Result<cdk02::KeySetInfo> {
        let url = self.base.join(&format!("/v1/keysets/{}", kid))?;
        let res = self.cl.get(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(kid));
        }
        let ks = res.json::<cdk02::KeySetInfo>().await?;
        Ok(ks)
    }

    pub async fn sign(&self, msg: &cdk00::BlindedMessage) -> Result<cdk00::BlindSignature> {
        let url = self.base.join("/v1/admin/keys/sign")?;
        let res = self.cl.post(url).json(msg).send().await?;
        if res.status() == reqwest::StatusCode::BAD_REQUEST {
            return Err(Error::InvalidRequest);
        }
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(msg.keyset_id));
        }
        let sig = res.json::<cdk00::BlindSignature>().await?;
        Ok(sig)
    }

    pub async fn verify(&self, proof: &cdk00::Proof) -> Result<bool> {
        let url = self.base.join("/v1/admin/keys/verify")?;
        let res = self.cl.post(url).json(proof).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(proof.keyset_id));
        }
        if res.status() == reqwest::StatusCode::BAD_REQUEST {
            return Ok(false);
        }
        res.error_for_status()?;
        Ok(true)
    }

    pub async fn pre_sign(
        &self,
        kid: cdk02::Id,
        qid: uuid::Uuid,
        tstamp: chrono::DateTime<chrono::Utc>,
        msg: &cdk00::BlindedMessage,
    ) -> Result<cdk00::BlindSignature> {
        let url = self.base.join("/v1/admin/keys/pre_sign")?;
        let msg = web_keys::PreSignRequest {
            kid,
            qid,
            expire: tstamp,
            msg: msg.clone(),
        };
        let res = self.cl.post(url).json(&msg).send().await?;
        if res.status() == reqwest::StatusCode::BAD_REQUEST {
            return Err(Error::InvalidRequest);
        }
        let sig = res.json::<cdk00::BlindSignature>().await?;
        Ok(sig)
    }
}
