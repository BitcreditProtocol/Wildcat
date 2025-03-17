// ----- standard library imports
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
// ----- extra library imports
use async_trait::async_trait;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut02 as cdk02;
use uuid::Uuid;
// ----- local imports
use crate::error::Result;
use crate::service::Repository;

#[derive(Clone, Default, Debug)]
pub struct InMemoryRepository {
    counters: Arc<Mutex<HashMap<cdk02::Id, u32>>>,
    secrets: Arc<Mutex<HashMap<Uuid, cdk00::PreMintSecrets>>>,
    signatures: Arc<Mutex<HashMap<Uuid, Vec<cdk00::BlindSignature>>>>,
}

#[async_trait]
impl Repository for InMemoryRepository {
    async fn next_counter(&self, kid: cdk02::Id) -> Result<u32> {
        let val = self
            .counters
            .lock()
            .unwrap()
            .get(&kid)
            .copied()
            .unwrap_or_default();
        Ok(val)
    }

    async fn increment_counter(&self, kid: cdk02::Id, inc: u32) -> Result<()> {
        let mut map = self.counters.lock().unwrap();
        let val = map.get(&kid).copied().unwrap_or_default() + inc;
        map.insert(kid, val);
        Ok(())
    }

    async fn store_secrets(&self, rid: Uuid, premint: cdk00::PreMintSecrets) -> Result<()> {
        self.secrets.lock().unwrap().insert(rid, premint);
        Ok(())
    }

    async fn store_signatures(
        &self,
        rid: Uuid,
        signatures: Vec<cdk00::BlindSignature>,
    ) -> Result<()> {
        self.signatures.lock().unwrap().insert(rid, signatures);
        Ok(())
    }
}
