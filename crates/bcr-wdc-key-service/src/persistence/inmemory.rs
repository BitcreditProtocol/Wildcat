// ----- standard library imports
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_utils::keys::KeysetEntry;
use cashu::nuts::nut02 as cdk02;
use cdk_common::mint as cdk_mint;
use cdk_common::mint::MintKeySetInfo;
// ----- local imports
use crate::error::{Error, Result};
use crate::service::{KeysRepository, MintCondition, QuoteKeysRepository};

#[derive(Default, Debug, Clone)]
pub struct InMemoryMap {
    keys: Arc<RwLock<HashMap<cdk02::Id, (KeysetEntry, MintCondition)>>>,
}

impl InMemoryMap {
    pub fn info(&self, kid: &cdk02::Id) -> Result<Option<cdk_mint::MintKeySetInfo>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(kid)
            .map(|((info, _), _)| info)
            .cloned();
        Ok(a)
    }

    pub fn condition(&self, kid: &cdk02::Id) -> Result<Option<MintCondition>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(kid)
            .map(|(_, condition)| condition)
            .cloned();
        Ok(a)
    }

    pub fn mark_as_minted(&self, kid: &cdk02::Id) -> Result<()> {
        let mut locked = self.keys.write().unwrap();
        if let Some((_, condition)) = locked.get_mut(kid) {
            if condition.is_minted {
                return Err(Error::InvalidMintRequest);
            }
            condition.is_minted = true;
            return Ok(());
        }
        Err(Error::UnknownKeyset(*kid))
    }

    pub fn list_info(&self) -> Result<Vec<cdk_mint::MintKeySetInfo>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .iter()
            .map(|(_, ((info, _), _))| info)
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
            .map(|((_, keyset), _)| keyset)
            .cloned();
        Ok(a)
    }

    pub fn list_keyset(&self) -> Result<Vec<cdk02::MintKeySet>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .iter()
            .map(|(_, ((_, keyset), _))| keyset)
            .cloned()
            .collect();
        Ok(a)
    }

    pub fn store(&self, entry: KeysetEntry, condition: MintCondition) -> Result<()> {
        self.keys
            .write()
            .unwrap()
            .insert(entry.0.id, (entry, condition));
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
    async fn condition(&self, kid: &cdk02::Id) -> Result<Option<MintCondition>> {
        self.condition(kid)
    }
    async fn store(&self, entry: KeysetEntry, cond: MintCondition) -> Result<()> {
        self.store(entry, cond)
    }
    async fn mark_as_minted(&self, kid: &cdk02::Id) -> Result<()> {
        self.mark_as_minted(kid)
    }
}

#[derive(Default, Clone)]
pub struct InMemoryQuoteKeyMap {
    keys: Arc<RwLock<HashMap<uuid::Uuid, (KeysetEntry, MintCondition)>>>,
}

#[async_trait]
impl QuoteKeysRepository for InMemoryQuoteKeyMap {
    async fn info(&self, qid: &uuid::Uuid) -> Result<Option<MintKeySetInfo>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(qid)
            .map(|((info, _), _)| info.clone());
        Ok(a)
    }
    async fn keyset(&self, qid: &uuid::Uuid) -> Result<Option<cdk02::MintKeySet>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(qid)
            .map(|((_, keyset), _)| keyset.clone());
        Ok(a)
    }
    async fn condition(&self, qid: &uuid::Uuid) -> Result<Option<MintCondition>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(qid)
            .map(|(_, condition)| condition.clone());
        Ok(a)
    }
    async fn entry(&self, qid: &uuid::Uuid) -> Result<Option<KeysetEntry>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(qid)
            .map(|(entry, _)| entry.clone());
        Ok(a)
    }
    async fn store(
        &self,
        qid: &uuid::Uuid,
        entry: KeysetEntry,
        condition: MintCondition,
    ) -> Result<()> {
        self.keys.write().unwrap().insert(*qid, (entry, condition));
        Ok(())
    }
}
