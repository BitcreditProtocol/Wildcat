// ----- standard library imports
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_keys::KeysetEntry;
use cashu::mint as cdk_mint;
use cashu::mint::MintKeySetInfo;
use cashu::nuts::nut02 as cdk02;
// ----- local imports
use crate::error::Result;
use crate::service::{KeysRepository, QuoteKeysRepository};

#[derive(Default, Clone)]
pub struct InMemoryMap {
    keys: Arc<RwLock<HashMap<cdk02::Id, KeysetEntry>>>,
}

#[async_trait]
impl KeysRepository for InMemoryMap {
    async fn info(&self, kid: &cdk02::Id) -> Result<Option<cdk_mint::MintKeySetInfo>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(kid)
            .map(|(info, _)| info.clone());
        Ok(a)
    }
    async fn keyset(&self, kid: &cdk02::Id) -> Result<Option<cdk02::MintKeySet>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(kid)
            .map(|(_, keyset)| keyset.clone());
        Ok(a)
    }

    async fn store(&self, entry: KeysetEntry) -> Result<()> {
        self.keys.write().unwrap().insert(entry.0.id, entry);
        Ok(())
    }
}

type QuoteKeysKey = (cdk02::Id, uuid::Uuid);
#[derive(Default, Clone)]
pub struct InMemoryQuoteKeyMap {
    keys: Arc<RwLock<HashMap<QuoteKeysKey, KeysetEntry>>>,
}

#[async_trait]
impl QuoteKeysRepository for InMemoryQuoteKeyMap {
    async fn info(&self, kid: &cdk02::Id, qid: &uuid::Uuid) -> Result<Option<MintKeySetInfo>> {
        let key = (*kid, *qid);
        let a = self
            .keys
            .read()
            .unwrap()
            .get(&key)
            .map(|(info, _)| info.clone());
        Ok(a)
    }
    async fn keyset(&self, kid: &cdk02::Id, qid: &uuid::Uuid) -> Result<Option<cdk02::MintKeySet>> {
        let key = (*kid, *qid);
        let a = self
            .keys
            .read()
            .unwrap()
            .get(&key)
            .map(|(_, keyset)| keyset.clone());
        Ok(a)
    }
    async fn store(&self, qid: &uuid::Uuid, entry: KeysetEntry) -> Result<()> {
        let key = (entry.0.id, *qid);
        self.keys.write().unwrap().insert(key, entry);
        Ok(())
    }
}
