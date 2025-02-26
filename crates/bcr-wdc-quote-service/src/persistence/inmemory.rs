// ----- standard library imports
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bcr_wdc_keys as keys;
use cashu::mint as cdk_mint;
use cashu::nuts::nut02 as cdk02;
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::keys::{KeysetEntry, KeysetID, Repository};
use crate::quotes;
use crate::TStamp;

#[derive(Default, Clone)]
pub struct QuotesIDMap {
    quotes: Arc<RwLock<HashMap<Uuid, quotes::Quote>>>,
}
#[async_trait]
impl quotes::Repository for QuotesIDMap {
    async fn search_by_bill(&self, bill: &str, endorser: &str) -> AnyResult<Vec<quotes::Quote>> {
        Ok(self
            .quotes
            .read()
            .unwrap()
            .iter()
            .filter(|quote| quote.1.bill.id == bill && quote.1.bill.holder.node_id == endorser)
            .map(|x| x.1.clone())
            .collect())
    }

    async fn store(&self, quote: quotes::Quote) -> AnyResult<()> {
        self.quotes.write().unwrap().insert(quote.id, quote);
        Ok(())
    }
    async fn load(&self, id: uuid::Uuid) -> AnyResult<Option<quotes::Quote>> {
        Ok(self.quotes.read().unwrap().get(&id).cloned())
    }

    async fn update_if_pending(&self, new: quotes::Quote) -> AnyResult<()> {
        let id = new.id;
        let mut m = self.quotes.write().unwrap();
        let result = m.remove(&id);
        if let Some(old) = result {
            if matches!(old.status, quotes::QuoteStatus::Pending { .. }) {
                m.insert(id, new);
            } else {
                m.insert(id, old);
            }
        }
        Ok(())
    }

    async fn update_if_offered(&self, new: quotes::Quote) -> AnyResult<()> {
        let id = new.id;
        let mut m = self.quotes.write().unwrap();
        let result = m.remove(&id);
        if let Some(old) = result {
            if matches!(old.status, quotes::QuoteStatus::Offered { .. }) {
                m.insert(id, new);
            } else {
                m.insert(id, old);
            }
        }
        Ok(())
    }

    async fn list_pendings(&self, since: Option<TStamp>) -> AnyResult<Vec<Uuid>> {
        let a = self
            .quotes
            .read()
            .unwrap()
            .iter()
            .filter(|(_, q)| matches!(q.status, quotes::QuoteStatus::Pending { .. }))
            .filter(|(_, q)| q.submitted >= since.unwrap_or_default())
            .map(|(id, _)| *id)
            .collect();
        Ok(a)
    }
    async fn list_offers(&self, _since: Option<TStamp>) -> AnyResult<Vec<Uuid>> {
        let a = self
            .quotes
            .read()
            .unwrap()
            .iter()
            .filter(|(_, q)| matches!(q.status, quotes::QuoteStatus::Accepted { .. }))
            .map(|(id, _)| *id)
            .collect();
        Ok(a)
    }
}

type QuoteKeysIndex = (KeysetID, Uuid);

#[derive(Default, Clone)]
pub struct KeysetIDQuoteIDMap {
    keys: Arc<RwLock<HashMap<QuoteKeysIndex, KeysetEntry>>>,
}

#[async_trait]
impl crate::keys_factory::QuoteBasedRepository for KeysetIDQuoteIDMap {
    async fn store(
        &self,
        qid: Uuid,
        keyset: cdk02::MintKeySet,
        info: cdk_mint::MintKeySetInfo,
    ) -> AnyResult<()> {
        self.keys
            .write()
            .unwrap()
            .insert((KeysetID::from(keyset.id), qid), (info, keyset));
        Ok(())
    }

    async fn load(&self, kid: &keys::KeysetID, qid: Uuid) -> AnyResult<Option<keys::KeysetEntry>> {
        let mapkey = (kid.clone(), qid);
        Ok(self.keys.read().unwrap().get(&mapkey).cloned())
    }
}

#[derive(Default, Clone)]
pub struct KeysetIDEntryMap {
    keys: Arc<RwLock<HashMap<KeysetID, KeysetEntry>>>,
}

#[async_trait]
impl keys::Repository for KeysetIDEntryMap {
    async fn info(&self, kid: &KeysetID) -> AnyResult<Option<cdk_mint::MintKeySetInfo>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(kid)
            .map(|(info, _)| info.clone());
        Ok(a)
    }
    async fn keyset(&self, kid: &KeysetID) -> AnyResult<Option<cdk02::MintKeySet>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(kid)
            .map(|(_, keyset)| keyset.clone());
        Ok(a)
    }
    async fn load(&self, kid: &KeysetID) -> AnyResult<Option<keys::KeysetEntry>> {
        let a = self.keys.read().unwrap().get(kid).cloned();
        Ok(a)
    }
    async fn store(
        &self,
        keyset: cdk02::MintKeySet,
        info: cdk_mint::MintKeySetInfo,
    ) -> AnyResult<()> {
        self.keys
            .write()
            .unwrap()
            .insert(KeysetID::from(keyset.id), (info, keyset));
        Ok(())
    }
}

#[derive(Default, Clone)]
pub struct KeysetIDEntryMapWithActive {
    keys: KeysetIDEntryMap,
    active: Arc<RwLock<Option<KeysetID>>>,
}

#[async_trait]
impl keys::Repository for KeysetIDEntryMapWithActive {
    async fn info(&self, kid: &KeysetID) -> AnyResult<Option<cdk_mint::MintKeySetInfo>> {
        self.keys.info(kid).await
    }

    async fn keyset(&self, kid: &KeysetID) -> AnyResult<Option<cdk02::MintKeySet>> {
        self.keys.keyset(kid).await
    }

    async fn load(&self, kid: &KeysetID) -> AnyResult<Option<KeysetEntry>> {
        self.keys.load(kid).await
    }

    async fn store(
        &self,
        keyset: cdk02::MintKeySet,
        info: cdk_mint::MintKeySetInfo,
    ) -> AnyResult<()> {
        if info.active {
            *self.active.write().unwrap() = Some(KeysetID::from(keyset.id));
        }
        self.keys.store(keyset, info).await
    }
}

#[async_trait]
impl keys::ActiveRepository for KeysetIDEntryMapWithActive {
    async fn info_active(&self) -> AnyResult<Option<cdk_mint::MintKeySetInfo>> {
        let kid = *self.active.read().unwrap();
        if let Some(kid) = kid {
            return self.keys.info(&kid).await;
        }
        Ok(None)
    }

    async fn keyset_active(&self) -> AnyResult<Option<cdk02::MintKeySet>> {
        let kid = *self.active.read().unwrap();
        if let Some(kid) = kid {
            return self.keys.keyset(&kid).await;
        }
        Ok(None)
    }
}
