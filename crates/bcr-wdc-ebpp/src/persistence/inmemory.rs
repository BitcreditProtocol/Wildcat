// ----- standard library imports
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
// ----- extra library imports
use async_trait::async_trait;
use uuid::Uuid;
// ----- local imports
use crate::error::{Error, Result};
use crate::onchain::{PrivateKeysRepository, SingleSecretKeyDescriptor};
use crate::payment::Request;
use crate::service::PaymentRepository;

// ----- end imports

#[derive(Default, Debug, Clone)]
pub struct InMemoryKeys {
    keys: Arc<Mutex<Vec<SingleSecretKeyDescriptor>>>,
}

#[async_trait]
impl PrivateKeysRepository for InMemoryKeys {
    async fn get_private_keys(&self) -> Result<Vec<SingleSecretKeyDescriptor>> {
        let locked = self.keys.lock().expect("get_private_keys");
        Ok(locked.clone())
    }

    async fn add_key(&self, key: SingleSecretKeyDescriptor) -> Result<()> {
        let mut locked = self.keys.lock().expect("add_key");
        locked.push(key);
        Ok(())
    }
}

pub struct InMemoryPaymentRepo {
    payments: Arc<Mutex<HashMap<Uuid, Request>>>,
}

#[async_trait]
impl PaymentRepository for InMemoryPaymentRepo {
    async fn load_request(&self, reqid: Uuid) -> Result<Request> {
        let locked = self.payments.lock().expect("load_request");
        if let Some(req) = locked.get(&reqid) {
            Ok(req.clone())
        } else {
            Err(Error::PaymentRequestNotFound(reqid))
        }
    }
    async fn store_request(&self, req: Request) -> Result<()> {
        let mut locked = self.payments.lock().expect("store_request");
        locked.insert(req.reqid, req);
        Ok(())
    }
    async fn update_request(&self, req: Request) -> Result<()> {
        let mut locked = self.payments.lock().expect("update_request");

        if let Some(existing_req) = locked.get_mut(&req.reqid) {
            *existing_req = req;
            Ok(())
        } else {
            Err(Error::PaymentRequestNotFound(req.reqid))
        }
    }
}
