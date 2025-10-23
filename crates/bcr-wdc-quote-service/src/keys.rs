// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::client::keys::Client as KeysClient;
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::{
    error::{Error, Result},
    service::KeysHandler,
};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct KeysRestConfig {
    pub base_url: reqwest::Url,
}

#[derive(Debug, Clone)]
pub struct KeysRestHandler(KeysClient);

impl KeysRestHandler {
    pub fn new(cfg: KeysRestConfig) -> Self {
        let cl = KeysClient::new(cfg.base_url);
        Self(cl)
    }
}

#[async_trait]
impl KeysHandler for KeysRestHandler {
    async fn get_keyset_with_redemption_date(
        &self,
        redemption_date: chrono::NaiveDate,
    ) -> Result<cashu::Id> {
        self.0
            .keys_for_expiration(redemption_date)
            .await
            .map_err(Error::KeysHandler)
    }
    async fn add_new_mint_operation(
        &self,
        qid: Uuid,
        kid: cashu::Id,
        pk: cashu::PublicKey,
        target: cashu::Amount,
    ) -> Result<()> {
        self.0
            .new_mint_operation(qid, kid, pk, target)
            .await
            .map_err(Error::KeysHandler)
    }
    async fn sign(&self, msg: &cashu::BlindedMessage) -> Result<cashu::BlindSignature> {
        self.0.sign(msg).await.map_err(Error::KeysHandler)
    }
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;
    use crate::TStamp;
    use bcr_wdc_utils::keys::KeysetEntry;
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    #[derive(Clone, Debug, Default)]
    pub struct DummyKeysHandler {
        keys: Arc<Mutex<HashMap<cashu::Id, KeysetEntry>>>,
    }

    #[async_trait]
    impl KeysHandler for DummyKeysHandler {
        async fn get_keyset_with_redemption_date(
            &self,
            redemption_date: chrono::NaiveDate,
        ) -> Result<cashu::Id> {
            let mut locked = self.keys.lock().unwrap();
            for (kid, (info, _)) in locked.iter() {
                let key_expiry = info.final_expiry.unwrap_or_default() as i64;
                let exp = TStamp::from_timestamp(key_expiry, 0).unwrap_or_default();
                if exp.date_naive() == redemption_date {
                    return Ok(*kid);
                }
            }
            let mut keysentry = bcr_wdc_utils::keys::test_utils::generate_keyset();
            let kid = keysentry.0.id;
            keysentry.0.final_expiry = Some(
                redemption_date
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc()
                    .timestamp() as u64,
            );
            keysentry.1.final_expiry = keysentry.0.final_expiry;
            locked.insert(kid, keysentry.clone());
            Ok(kid)
        }
        async fn add_new_mint_operation(
            &self,
            _qid: Uuid,
            _kid: cashu::Id,
            _pk: cashu::PublicKey,
            _target: cashu::Amount,
        ) -> Result<()> {
            Ok(())
        }
        async fn sign(&self, msg: &cashu::BlindedMessage) -> Result<cashu::BlindSignature> {
            let locked = self.keys.lock().unwrap();
            let (_, keyset) = locked.get(&msg.keyset_id).expect("Keyset not found");
            let sig = bcr_wdc_utils::keys::sign_with_keys(keyset, msg).expect("sign_with_keys");
            Ok(sig)
        }
    }
}
