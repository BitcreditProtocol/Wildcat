// ----- standard library imports
// ----- extra library imports
use bcr_common::wire::keys as wire_keys;
use thiserror::Error;
// ----- local imports
pub use reqwest::Url;

// ----- end imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("resource not found {0}")]
    ResourceNotFound(cashu::Id),
    #[error("resource from id not found {0}")]
    ResourceFromIdNotFound(uuid::Uuid),
    #[error("invalid request")]
    InvalidRequest,

    #[error("internal error {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("sign error [{0}")]
    NUT20(#[from] cashu::nut20::Error),
}

#[derive(Debug, Clone)]
pub struct KeyClient {
    cl: reqwest::Client,
    base: reqwest::Url,
    #[cfg(feature = "authorized")]
    auth: bcr_wdc_utils::client::AuthorizationPlugin,
}

impl KeyClient {
    pub fn new(base: reqwest::Url) -> Self {
        Self {
            cl: reqwest::Client::new(),
            base,
            #[cfg(feature = "authorized")]
            auth: Default::default(),
        }
    }

    #[cfg(feature = "authorized")]
    pub async fn authenticate(
        &mut self,
        token_url: Url,
        client_id: &str,
        client_secret: &str,
        username: &str,
        password: &str,
    ) -> Result<()> {
        self.auth
            .authenticate(
                self.cl.clone(),
                token_url,
                client_id,
                client_secret,
                username,
                password,
            )
            .await?;
        Ok(())
    }

    pub async fn keys(&self, kid: cashu::Id) -> Result<cashu::KeySet> {
        let url = self
            .base
            .join(&format!("/v1/keys/{kid}"))
            .expect("keys relative path");
        let res = self.cl.get(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(kid));
        }
        let mut ks = res.json::<cashu::KeysResponse>().await?;
        if ks.keysets.len() != 1 {
            return Err(Error::ResourceNotFound(kid));
        }
        Ok(ks.keysets.remove(0))
    }

    pub async fn list_keys(&self) -> Result<Vec<cashu::KeySet>> {
        let url = self.base.join("/v1/keys").expect("keys relative path");
        let res = self.cl.get(url).send().await?;
        let ks = res.json::<cashu::KeysResponse>().await?;
        Ok(ks.keysets)
    }

    pub async fn keyset_info(&self, kid: cashu::Id) -> Result<cashu::KeySetInfo> {
        let url = self
            .base
            .join(&format!("/v1/keysets/{kid}"))
            .expect("keyset relative path");
        let res = self.cl.get(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(kid));
        }
        let ks = res.json::<cashu::KeySetInfo>().await?;
        Ok(ks)
    }

    pub async fn list_keyset_info(&self) -> Result<Vec<cashu::KeySetInfo>> {
        let url = self.base.join("/v1/keysets").expect("keyset relative path");
        let res = self.cl.get(url).send().await?;
        let ks = res.json::<cashu::KeysetResponse>().await?;
        Ok(ks.keysets)
    }

    #[cfg(feature = "authorized")]
    pub async fn sign(&self, msg: &cashu::BlindedMessage) -> Result<cashu::BlindSignature> {
        let url = self
            .base
            .join("/v1/admin/keys/sign")
            .expect("sign relative path");
        let request = self.cl.post(url).json(msg);
        let response = self.auth.authorize(request).send().await?;
        if response.status() == reqwest::StatusCode::BAD_REQUEST {
            return Err(Error::InvalidRequest);
        }
        if response.status() == reqwest::StatusCode::CONFLICT {
            return Err(Error::InvalidRequest);
        }
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(msg.keyset_id));
        }
        let sig = response.json::<cashu::BlindSignature>().await?;
        Ok(sig)
    }

    #[cfg(feature = "authorized")]
    pub async fn verify(&self, proof: &cashu::Proof) -> Result<()> {
        let url = self
            .base
            .join("/v1/admin/keys/verify")
            .expect("verify relative path");
        let request = self.cl.post(url).json(proof);
        let response = self.auth.authorize(request).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(proof.keyset_id));
        }
        if response.status() == reqwest::StatusCode::BAD_REQUEST {
            return Err(Error::InvalidRequest);
        }
        response.error_for_status()?;
        Ok(())
    }

    #[cfg(feature = "authorized")]
    pub async fn keys_for_expiration(&self, date: chrono::NaiveDate) -> Result<cashu::Id> {
        let url = self
            .base
            .join(&format!("/v1/admin/keys/{date}"))
            .expect("keys for date relative path");
        let request = self.cl.get(url);
        let res = self.auth.authorize(request).send().await?;
        let kid = res.json::<cashu::Id>().await?;
        Ok(kid)
    }

    #[cfg(feature = "authorized")]
    pub async fn new_mint_operation(
        &self,
        qid: uuid::Uuid,
        kid: cashu::Id,
        pk: cashu::PublicKey,
        target: cashu::Amount,
    ) -> Result<()> {
        let url = self
            .base
            .join("/v1/admin/keys/mintop")
            .expect("mint operation relative path");
        let msg = wire_keys::NewMintOperationRequest {
            quote_id: qid,
            kid,
            pub_key: pk,
            target,
        };
        let result = self.cl.post(url).json(&msg).send().await?;
        if result.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(kid));
        }
        let _response = result.json::<wire_keys::NewMintOperationResponse>().await?;
        Ok(())
    }

    pub async fn mint(
        &self,
        qid: uuid::Uuid,
        outputs: Vec<cashu::BlindedMessage>,
        sk: cashu::SecretKey,
    ) -> Result<Vec<cashu::BlindSignature>> {
        let url = self
            .base
            .join("/v1/mint/ebill")
            .expect("mint relative path");
        let mut msg = cashu::MintRequest {
            quote: qid,
            outputs,
            signature: None,
        };
        msg.sign(sk)?;
        let result = self.cl.post(url).json(&msg).send().await?;
        if result.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceFromIdNotFound(qid));
        }
        let response = result.json::<cashu::MintResponse>().await?;
        Ok(response.signatures)
    }

    pub async fn restore(
        &self,
        outputs: Vec<cashu::BlindedMessage>,
    ) -> Result<Vec<(cashu::BlindedMessage, cashu::BlindSignature)>> {
        let url = self.base.join("v1/restore").expect("restore relative path");
        let msg = cashu::RestoreRequest { outputs };
        let response = self.cl.post(url).json(&msg).send().await?;
        let msg: cashu::RestoreResponse = response.json().await?;
        let cashu::RestoreResponse {
            outputs,
            signatures,
            ..
        } = msg;
        let ret_val = outputs
            .into_iter()
            .zip(signatures.into_iter())
            .collect::<Vec<_>>();
        Ok(ret_val)
    }

    #[cfg(feature = "authorized")]
    pub async fn deactivate_keyset(&self, kid: cashu::Id) -> Result<cashu::Id> {
        let url = self
            .base
            .join("/v1/admin/keys/deactivate")
            .expect("deactivate relative path");
        let msg = wire_keys::DeactivateKeysetRequest { kid };
        let request = self.cl.post(url).json(&msg);
        let response = self.auth.authorize(request).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(kid));
        }
        let response: wire_keys::DeactivateKeysetResponse = response.json().await?;
        Ok(response.kid)
    }
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;
    use bcr_wdc_key_service::KeysRepository;

    #[derive(Debug, Default, Clone)]
    pub struct KeyClient {
        pub keys: bcr_wdc_key_service::test_utils::InMemoryRepository,
    }

    impl KeyClient {
        pub async fn keyset(&self, kid: cashu::Id) -> Result<cashu::KeySet> {
            let res = self.keys.keyset(kid).await.expect("InMemoryRepository");
            res.ok_or(Error::ResourceNotFound(kid))
                .map(std::convert::Into::into)
        }
        pub async fn list_keyset(&self) -> Result<Vec<cashu::KeySet>> {
            let res = self.keys.list_keyset().await.expect("InMemoryRepository");
            let ret = res.into_iter().map(cashu::KeySet::from).collect();
            Ok(ret)
        }
        pub async fn keyset_info(&self, kid: cashu::Id) -> Result<cashu::KeySetInfo> {
            self.keys
                .info(kid)
                .await
                .expect("InMemoryRepository")
                .ok_or(Error::ResourceNotFound(kid))
                .map(std::convert::Into::into)
        }
        pub async fn list_keyset_info(&self) -> Result<Vec<cashu::KeySetInfo>> {
            let res = self.keys.list_info().await.expect("InMemoryRepository");
            let ret = res.into_iter().map(cashu::KeySetInfo::from).collect();
            Ok(ret)
        }
        pub async fn sign(&self, msg: &cashu::BlindedMessage) -> Result<cashu::BlindSignature> {
            let res = self
                .keys
                .keyset(msg.keyset_id)
                .await
                .expect("InMemoryRepository");
            let keys = res.ok_or(Error::ResourceNotFound(msg.keyset_id))?;
            bcr_wdc_utils::keys::sign_with_keys(&keys, msg).map_err(|_| Error::InvalidRequest)
        }
        pub async fn verify(&self, proof: &cashu::Proof) -> Result<bool> {
            let res = self
                .keys
                .keyset(proof.keyset_id)
                .await
                .expect("InMemoryRepository");
            let keys = res.ok_or(Error::ResourceNotFound(proof.keyset_id))?;
            bcr_wdc_utils::keys::verify_with_keys(&keys, proof)
                .map_err(|_| Error::InvalidRequest)?;
            Ok(true)
        }
        pub async fn pre_sign(
            &self,
            _qid: uuid::Uuid,
            _msg: &cashu::BlindedMessage,
        ) -> Result<cashu::BlindSignature> {
            todo!()
        }

        pub async fn generate_keyset(
            &self,
            _qid: uuid::Uuid,
            _target: cashu::Amount,
            _pub_key: cashu::PublicKey,
            _expire: chrono::DateTime<chrono::Utc>,
        ) -> Result<cashu::Id> {
            todo!();
        }

        pub async fn mint(
            &self,
            _outputs: &[cashu::BlindedMessage],
            _sk: cashu::SecretKey,
        ) -> Result<()> {
            todo!()
        }

        pub async fn restore(
            &self,
            _outputs: Vec<cashu::BlindedMessage>,
        ) -> Result<Vec<(cashu::BlindedMessage, cashu::BlindSignature)>> {
            todo!()
        }
    }
}
