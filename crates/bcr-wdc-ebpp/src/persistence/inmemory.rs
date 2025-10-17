// ----- standard library imports
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
// ----- extra library imports
use async_trait::async_trait;
use cashu::MintQuoteState;
use uuid::Uuid;
// ----- local imports
use crate::onchain::{PrivateKeysRepository, SingleSecretKeyDescriptor};
use crate::payment::IncomingRequest;
use crate::service::PaymentRepository;
use crate::{
    error::{Error, Result},
    payment::OutgoingRequest,
};

// ----- end imports

#[allow(dead_code)]
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

#[allow(dead_code)]
pub struct InMemoryPaymentRepo {
    incomings: Arc<Mutex<HashMap<Uuid, IncomingRequest>>>,
    outgoings: Arc<Mutex<HashMap<Uuid, OutgoingRequest>>>,
    foreign: Arc<Mutex<Vec<(Uuid, String)>>>,
}

#[async_trait]
impl PaymentRepository for InMemoryPaymentRepo {
    async fn load_incoming(&self, reqid: Uuid) -> Result<IncomingRequest> {
        let locked = self.incomings.lock().expect("load_incoming");
        if let Some(req) = locked.get(&reqid) {
            Ok(req.clone())
        } else {
            Err(Error::PaymentRequestNotFound(reqid))
        }
    }
    async fn store_incoming(&self, req: IncomingRequest) -> Result<()> {
        let mut locked = self.incomings.lock().expect("store_incoming");
        locked.insert(req.reqid, req);
        Ok(())
    }
    async fn update_incoming(&self, req: IncomingRequest) -> Result<()> {
        let mut locked = self.incomings.lock().expect("update_incoming");
        if let Some(existing_req) = locked.get_mut(&req.reqid) {
            *existing_req = req;
            Ok(())
        } else {
            Err(Error::PaymentRequestNotFound(req.reqid))
        }
    }

    async fn load_outgoing(&self, reqid: Uuid) -> Result<OutgoingRequest> {
        let locked = self.outgoings.lock().expect("load_outgoing");
        if let Some(req) = locked.get(&reqid) {
            Ok(req.clone())
        } else {
            Err(Error::PaymentRequestNotFound(reqid))
        }
    }

    async fn store_outgoing(&self, req: OutgoingRequest) -> Result<()> {
        let mut locked = self.outgoings.lock().expect("store_outgoing");
        locked.insert(req.reqid, req);
        Ok(())
    }

    async fn update_outgoing(&self, req: OutgoingRequest) -> Result<()> {
        let mut locked = self.outgoings.lock().expect("update_outgoing");
        if let Some(existing_req) = locked.get_mut(&req.reqid) {
            *existing_req = req;
            Ok(())
        } else {
            Err(Error::PaymentRequestNotFound(req.reqid))
        }
    }

    async fn list_unpaid_incoming_requests(&self) -> Result<Vec<IncomingRequest>> {
        let locked = self
            .incomings
            .lock()
            .expect("list_unpaid_incoming_requests");

        let values = locked
            .iter()
            .filter_map(|(_, v)| match v.status {
                MintQuoteState::Unpaid => Some(v),
                _ => None,
            })
            .cloned()
            .collect();
        Ok(values)
    }

    async fn store_foreign(&self, reqid: Uuid, nonce: String) -> Result<()> {
        let mut locked = self.foreign.lock().expect("store_foreign");
        locked.push((reqid, nonce));
        Ok(())
    }
    async fn check_foreign_nonce(&self, nonce: &str) -> Result<bool> {
        let locked = self.foreign.lock().expect("check_foreign_nonce");
        for (_, stored_nonce) in locked.iter() {
            if stored_nonce == nonce {
                return Ok(true);
            }
        }
        Ok(false)
    }
    async fn check_foreign_reqid(&self, reqid: Uuid) -> Result<bool> {
        let locked = self.foreign.lock().expect("check_foreign_reqid");
        for (stored_reqid, _) in locked.iter() {
            if *stored_reqid == reqid {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
