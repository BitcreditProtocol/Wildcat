// ----- standard library imports
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
// ----- extra library imports
use anyhow::Result as AnyResult;
use cdk::nuts::nut00 as cdk00;
use cdk::nuts::nut01 as cdk01;
use cdk::nuts::nut02 as cdk02;
use cdk::nuts::nut07 as cdk07;
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::credit::{keys, quotes};
use crate::keys::KeysetID;
use crate::swap;
use crate::TStamp;

#[derive(Default, Clone)]
pub struct QuoteRepo {
    quotes: Arc<RwLock<HashMap<Uuid, quotes::Quote>>>,
}
impl quotes::Repository for QuoteRepo {
    fn search_by_bill(&self, bill: &str, endorser: &str) -> AnyResult<Option<quotes::Quote>> {
        Ok(self
            .quotes
            .read()
            .unwrap()
            .iter()
            .find(|quote| quote.1.bill == bill && quote.1.endorser == endorser)
            .map(|(_, q)| q.clone()))
    }

    fn store(&self, quote: quotes::Quote) -> AnyResult<()> {
        self.quotes.write().unwrap().insert(quote.id, quote);
        Ok(())
    }
    fn load(&self, id: uuid::Uuid) -> AnyResult<Option<quotes::Quote>> {
        Ok(self.quotes.read().unwrap().get(&id).cloned())
    }

    fn update_if_pending(&self, new: quotes::Quote) -> AnyResult<()> {
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

    fn list_pendings(&self, since: Option<TStamp>) -> AnyResult<Vec<Uuid>> {
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
    fn list_accepteds(&self, _since: Option<TStamp>) -> AnyResult<Vec<Uuid>> {
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
type KeysetEntry = (cdk::mint::MintKeySetInfo, cdk02::MintKeySet);

#[derive(Default, Clone)]
pub struct QuoteKeysRepo {
    keys: Arc<RwLock<HashMap<QuoteKeysIndex, KeysetEntry>>>,
}

impl keys::QuoteKeyRepository for QuoteKeysRepo {
    fn store(
        &self,
        qid: Uuid,
        keyset: cdk02::MintKeySet,
        info: cdk::mint::MintKeySetInfo,
    ) -> AnyResult<()> {
        self.keys
            .write()
            .unwrap()
            .insert((KeysetID::from(keyset.id), qid), (info, keyset));
        Ok(())
    }
}

#[derive(Default, Clone)]
pub struct SimpleKeysRepo {
    keys: Arc<RwLock<HashMap<KeysetID, KeysetEntry>>>,
}

impl SimpleKeysRepo {
    fn keyinfo(&self, kid: &KeysetID) -> AnyResult<Option<cdk::mint::MintKeySetInfo>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(kid)
            .map(|(info, _)| info.clone());
        Ok(a)
    }
    fn keyset(&self, kid: &KeysetID) -> AnyResult<Option<cdk02::MintKeySet>> {
        let a = self
            .keys
            .read()
            .unwrap()
            .get(kid)
            .map(|(_, set)| set.clone());
        Ok(a)
    }
    fn store(&self, keyset: cdk02::MintKeySet, info: cdk::mint::MintKeySetInfo) -> AnyResult<()> {
        self.keys
            .write()
            .unwrap()
            .insert(KeysetID::from(keyset.id), (info, keyset));
        Ok(())
    }
}

impl keys::MaturityKeyRepository for SimpleKeysRepo {
    fn info(&self, kid: &KeysetID) -> AnyResult<Option<cdk::mint::MintKeySetInfo>> {
        self.keyinfo(kid)
    }

    fn store(&self, keyset: cdk02::MintKeySet, info: cdk::mint::MintKeySetInfo) -> AnyResult<()> {
        self.store(keyset, info)
    }
}

impl swap::KeysRepository for SimpleKeysRepo {
    fn load(&self, id: &KeysetID) -> AnyResult<Option<cdk02::MintKeySet>> {
        self.keyset(id)
    }

    fn info(&self, id: &KeysetID) -> AnyResult<Option<cdk::mint::MintKeySetInfo>> {
        self.keyinfo(id)
    }

    fn replacing_id(&self, id: &KeysetID) -> AnyResult<Option<KeysetID>> {
        Ok(Some(id.clone()))
    }
}

#[derive(Default, Clone)]
pub struct ProofRepo {
    proofs: Arc<RwLock<HashMap<cdk01::PublicKey, cdk07::ProofState>>>,
}

impl swap::ProofRepository for ProofRepo {
    fn spend(&self, tokens: &[cdk00::Proof]) -> AnyResult<()> {
        let mut writer = self.proofs.write().unwrap();
        for token in tokens {
            let y = cdk::dhke::hash_to_curve(&token.secret.to_bytes())?;
            writer.insert(
                y,
                cdk07::ProofState {
                    y,
                    state: cdk07::State::Spent,
                    witness: None,
                },
            );
        }
        Ok(())
    }

    fn get_state(&self, tokens: &[cdk00::Proof]) -> AnyResult<Vec<cdk07::State>> {
        let mut states: Vec<cdk07::State> = Vec::with_capacity(tokens.len());
        let reader = self.proofs.read().unwrap();
        for token in tokens {
            let y = cdk::dhke::hash_to_curve(&token.secret.to_bytes())?;
            let state = reader.get(&y).map_or(cdk07::State::Unspent, |s| s.state);
            states.push(state);
        }
        Ok(states)
    }
}
