// ----- standard library imports
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_keys::KeysetEntry;
use cashu::nuts::nut02 as cdk02;
use cdk_common::mint as cdk_mint;
use cdk_common::mint::MintKeySetInfo;
// ----- local imports
use crate::error::Result;
use crate::service::{KeysRepository, QuoteKeysRepository};

#[derive(Default, Debug, Clone)]
pub struct InMemoryMap {
    keys: Arc<RwLock<HashMap<cdk02::Id, KeysetEntry>>>,
}

impl InMemoryMap {
    pub fn info(&self, kid: &cdk02::Id) -> Result<Option<cdk_mint::MintKeySetInfo>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(kid)
            .map(|(info, _)| info)
            .cloned();
        Ok(a)
    }

    pub fn list_info(&self) -> Result<Vec<cdk_mint::MintKeySetInfo>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .iter()
            .map(|(_, (info, _))| info)
            .cloned()
            .collect();
        Ok(a)
    }

    pub fn keyset(&self, kid: &cdk02::Id) -> Result<Option<cdk02::MintKeySet>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(kid)
            .map(|(_, keyset)| keyset)
            .cloned();
        Ok(a)
    }

    pub fn list_keyset(&self) -> Result<Vec<cdk02::MintKeySet>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .iter()
            .map(|(_, (_, keyset))| keyset)
            .cloned()
            .collect();
        Ok(a)
    }

    pub fn store(&self, entry: KeysetEntry) -> Result<()> {
        self.keys.write().unwrap().insert(entry.0.id, entry);
        Ok(())
    }
}

#[async_trait]
impl KeysRepository for InMemoryMap {
    async fn info(&self, kid: &cdk02::Id) -> Result<Option<cdk_mint::MintKeySetInfo>> {
        self.info(kid)
    }
    async fn list_info(&self) -> Result<Vec<MintKeySetInfo>> {
        self.list_info()
    }
    async fn keyset(&self, kid: &cdk02::Id) -> Result<Option<cdk02::MintKeySet>> {
        self.keyset(kid)
    }
    async fn list_keyset(&self) -> Result<Vec<cdk02::MintKeySet>> {
        self.list_keyset()
    }
    async fn store(&self, entry: KeysetEntry) -> Result<()> {
        self.store(entry)
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
