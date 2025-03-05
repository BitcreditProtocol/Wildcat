// ----- standard library imports
// ----- extra library imports
use cashu::nut02 as cdk02;
use thiserror::Error;
// ----- local modules
// ----- local imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("Url parse error {0}")]
    Url(#[from] url::ParseError),
    #[error("client error {0}")]
    Reqwest(#[from] reqwest::Error),
}

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
        let ks = res.json::<cdk02::KeySet>().await?;
        Ok(ks)
    }

    pub async fn keyset(&self, kid: cdk02::Id) -> Result<cdk02::KeySetInfo> {
        let url = self.base.join(&format!("/v1/keysets/{}", kid))?;
        let res = self.cl.get(url).send().await?;
        let ks = res.json::<cdk02::KeySetInfo>().await?;
        Ok(ks)
    }
}
