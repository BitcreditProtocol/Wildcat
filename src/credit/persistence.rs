// ----- standard library imports
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
// ----- extra library imports
use cdk::nuts::nut02 as cdk02;
use uuid::Uuid;
// ----- local modules
// ----- local imports
use super::{keys, quotes, TStamp};

#[derive(Default, Clone)]
pub struct InMemoryQuoteRepository {
    quotes: Arc<RwLock<HashMap<Uuid, quotes::Quote>>>,
}
impl quotes::Repository for InMemoryQuoteRepository {
    fn search_by(&self, bill: &str, endorser: &str) -> Option<quotes::Quote> {
        self.quotes
            .read()
            .unwrap()
            .values()
            .find(|q| q.bill == bill && q.endorser == endorser)
            .cloned()
    }
    fn store(&self, quote: quotes::Quote) -> std::result::Result<(), Box<dyn std::error::Error>> {
        self.quotes.write().unwrap().insert(quote.id, quote);
        Ok(())
    }
}

impl InMemoryQuoteRepository {
    pub fn load(&self, id: Uuid) -> Option<quotes::Quote> {
        self.quotes.read().unwrap().get(&id).cloned()
    }

    pub fn update_if_pending(&self, new: quotes::Quote) {
        let id = new.id;
        let mut m = self.quotes.write().unwrap();
        let result = m.remove(&id);
        if let Some(old) = result {
            if matches!(old.status(), quotes::QuoteStatus::Pending { .. }) {
                m.insert(id, new);
            } else {
                m.insert(id, old);
            }
        }
    }

    pub fn list_pendings(&self, since: Option<TStamp>) -> Vec<Uuid> {
        self.quotes
            .read()
            .unwrap()
            .iter()
            .filter(|(_, q)| matches!(q.status(), quotes::QuoteStatus::Pending { .. }))
            .filter(|(_, q)| q.submitted >= since.unwrap_or_default())
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn list_accepteds(&self) -> Vec<Uuid> {
        self.quotes
            .read()
            .unwrap()
            .iter()
            .filter(|(_, q)| matches!(q.status(), quotes::QuoteStatus::Accepted { .. }))
            .map(|(id, _)| *id)
            .collect()
    }
}

#[derive(Default, Clone)]
pub struct InMemoryKeysRepository {
    keys: Arc<RwLock<HashMap<keys::KeysetID, (cdk::mint::MintKeySetInfo, cdk02::MintKeySet)>>>,
}

impl keys::CreateRepository for InMemoryKeysRepository {
    fn info(&self, id: &keys::KeysetID) -> Option<cdk::mint::MintKeySetInfo> {
        self.keys.read().unwrap().get(id).map(|k| k.0.clone())
    }
    fn store(
        &self,
        keyset: cdk02::MintKeySet,
        info: cdk::mint::MintKeySetInfo,
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        self.keys
            .write()
            .unwrap()
            .insert(keys::KeysetID::from(keyset.id), (info, keyset));
        Ok(())
    }
}
