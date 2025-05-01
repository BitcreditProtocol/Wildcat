// ----- standard library imports
// ----- extra library imports
use bcr_wdc_webapi::keys as web_keys;
use cashu::{nut00 as cdk00, nut01 as cdk01, nut02 as cdk02};
use thiserror::Error;
// ----- local imports
pub use reqwest::Url;

// ----- end imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("resource not found {0}")]
    ResourceNotFound(cdk02::Id),
    #[error("resource from id not found {0}")]
    ResourceFromIdNotFound(uuid::Uuid),
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
    pub fn new(base: reqwest::Url) -> Self {
        Self {
            cl: reqwest::Client::new(),
            base,
        }
    }

    pub async fn keys(&self, kid: cdk02::Id) -> Result<cdk02::KeySet> {
        let url = self
            .base
            .join(&format!("/v1/keys/{}", kid))
            .expect("keys relative path");
        let res = self.cl.get(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(kid));
        }
        let ks = res.json::<cdk02::KeySet>().await?;
        Ok(ks)
    }

    pub async fn list_keys(&self) -> Result<Vec<cdk02::KeySet>> {
        let url = self.base.join("/v1/keys").expect("keys relative path");
        let res = self.cl.get(url).send().await?;
        let ks = res.json::<cdk01::KeysResponse>().await?;
        Ok(ks.keysets)
    }

    pub async fn keyset_info(&self, kid: cdk02::Id) -> Result<cdk02::KeySetInfo> {
        let url = self
            .base
            .join(&format!("/v1/keysets/{}", kid))
            .expect("keyset relative path");
        let res = self.cl.get(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(kid));
        }
        let ks = res.json::<cdk02::KeySetInfo>().await?;
        Ok(ks)
    }

    pub async fn list_keyset_info(&self) -> Result<Vec<cdk02::KeySetInfo>> {
        let url = self.base.join("/v1/keysets").expect("keyset relative path");
        let res = self.cl.get(url).send().await?;
        let ks = res.json::<cdk02::KeysetResponse>().await?;
        Ok(ks.keysets)
    }

    pub async fn sign(&self, msg: &cdk00::BlindedMessage) -> Result<cdk00::BlindSignature> {
        let url = self
            .base
            .join("/v1/admin/keys/sign")
            .expect("sign relative path");
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

    pub async fn verify(&self, proof: &cdk00::Proof) -> Result<()> {
        let url = self
            .base
            .join("/v1/admin/keys/verify")
            .expect("verify relative path");
        let res = self.cl.post(url).json(proof).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(proof.keyset_id));
        }
        if res.status() == reqwest::StatusCode::BAD_REQUEST {
            return Err(Error::InvalidRequest);
        }
        res.error_for_status()?;
        Ok(())
    }

    pub async fn pre_sign(
        &self,
        qid: uuid::Uuid,
        msg: &cdk00::BlindedMessage,
    ) -> Result<cdk00::BlindSignature> {
        let url = self
            .base
            .join("/v1/admin/keys/pre_sign")
            .expect("pre_sign relative path");
        let msg = web_keys::PreSignRequest {
            qid,
            msg: msg.clone(),
        };
        let res = self.cl.post(url).json(&msg).send().await?;
        if res.status() == reqwest::StatusCode::BAD_REQUEST {
            return Err(Error::InvalidRequest);
        }
        let sig = res.json::<cdk00::BlindSignature>().await?;
        Ok(sig)
    }

    pub async fn generate(
        &self,
        qid: uuid::Uuid,
        amount: cashu::Amount,
        public_key: cdk01::PublicKey,
        expire: chrono::DateTime<chrono::Utc>,
    ) -> Result<cdk02::Id> {
        let url = self
            .base
            .join("/v1/admin/keys/generate")
            .expect("generate relative path");
        let msg = web_keys::GenerateKeysetRequest {
            qid,
            condition: web_keys::KeysetMintCondition { amount, public_key },
            expire,
        };
        let res = self.cl.post(url).json(&msg).send().await?;
        if res.status() == reqwest::StatusCode::BAD_REQUEST {
            return Err(Error::InvalidRequest);
        }
        let kid = res.json::<cdk02::Id>().await?;
        Ok(kid)
    }

    pub async fn activate_keyset(&self, qid: uuid::Uuid) -> Result<()> {
        let url = self
            .base
            .join("/v1/admin/keys/activate")
            .expect("activate relative path");
        let msg = web_keys::ActivateKeysetRequest { qid };
        let res = self.cl.post(url).json(&msg).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceFromIdNotFound(qid));
        }
        res.error_for_status()?;
        Ok(())
    }
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;

    #[derive(Debug, Default, Clone)]
    pub struct KeyClient {
        pub keys: bcr_wdc_key_service::test_utils::InMemoryRepository,
    }

    impl KeyClient {
        pub async fn keyset(&self, kid: cdk02::Id) -> Result<cdk02::KeySet> {
            let res = self.keys.keyset(&kid).expect("InMemoryRepository");
            res.ok_or(Error::ResourceNotFound(kid))
                .map(std::convert::Into::into)
        }
        pub async fn list_keyset(&self) -> Result<Vec<cdk02::KeySet>> {
            let res = self.keys.list_keyset().expect("InMemoryRepository");
            let ret = res.into_iter().map(cdk02::KeySet::from).collect();
            Ok(ret)
        }
        pub async fn keyset_info(&self, kid: cdk02::Id) -> Result<cdk02::KeySetInfo> {
            self.keys
                .info(&kid)
                .expect("InMemoryRepository")
                .ok_or(Error::ResourceNotFound(kid))
                .map(std::convert::Into::into)
        }
        pub async fn list_keyset_info(&self) -> Result<Vec<cdk02::KeySetInfo>> {
            let res = self.keys.list_info().expect("InMemoryRepository");
            let ret = res.into_iter().map(cdk02::KeySetInfo::from).collect();
            Ok(ret)
        }
        pub async fn sign(&self, msg: &cdk00::BlindedMessage) -> Result<cdk00::BlindSignature> {
            let res = self
                .keys
                .keyset(&msg.keyset_id)
                .expect("InMemoryRepository");
            let keys = res.ok_or(Error::ResourceNotFound(msg.keyset_id))?;
            bcr_wdc_utils::keys::sign_with_keys(&keys, msg).map_err(|_| Error::InvalidRequest)
        }
        pub async fn verify(&self, proof: &cdk00::Proof) -> Result<bool> {
            let res = self
                .keys
                .keyset(&proof.keyset_id)
                .expect("InMemoryRepository");
            let keys = res.ok_or(Error::ResourceNotFound(proof.keyset_id))?;
            bcr_wdc_utils::keys::verify_with_keys(&keys, proof)
                .map_err(|_| Error::InvalidRequest)?;
            Ok(true)
        }
        pub async fn pre_sign(
            &self,
            _kid: cdk02::Id,
            _qid: uuid::Uuid,
            _tstamp: chrono::DateTime<chrono::Utc>,
            _msg: &cdk00::BlindedMessage,
        ) -> Result<cdk00::BlindSignature> {
            todo!()
        }
    }
}
