// ----- standard library imports
// ----- extra library imports
use bcr_common::client::keys::Error as KeysError;
// ----- local imports
use crate::KeysRepository;

// ----- end imports

pub type Result<T> = std::result::Result<T, KeysError>;
#[derive(Debug, Default, Clone)]
pub struct DummyClient {
    pub keys: crate::test_utils::InMemoryRepository,
}

impl DummyClient {
    pub async fn keyset(&self, kid: cashu::Id) -> Result<cashu::KeySet> {
        let res = self.keys.keyset(kid).await.expect("InMemoryRepository");
        res.ok_or(KeysError::KeysetIdNotFound(kid))
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
            .ok_or(KeysError::KeysetIdNotFound(kid))
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
        let keys = res.ok_or(KeysError::KeysetIdNotFound(msg.keyset_id))?;
        bcr_wdc_utils::keys::sign_with_keys(&keys, msg).map_err(|_| KeysError::InvalidRequest)
    }
    pub async fn verify(&self, proof: &cashu::Proof) -> Result<bool> {
        let res = self
            .keys
            .keyset(proof.keyset_id)
            .await
            .expect("InMemoryRepository");
        let keys = res.ok_or(KeysError::KeysetIdNotFound(proof.keyset_id))?;
        bcr_wdc_utils::keys::verify_with_keys(&keys, proof)
            .map_err(|_| KeysError::InvalidRequest)?;
        Ok(true)
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
