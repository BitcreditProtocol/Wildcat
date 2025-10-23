// ----- standard library imports
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
// ----- extra library imports
use async_trait::async_trait;
use cashu::MintQuoteState;
use cdk_common::payment::PaymentIdentifier;
// ----- local imports
use crate::{
    error::{Error, Result},
    onchain::{PrivateKeysRepository, SingleSecretKeyDescriptor},
    payment::{ForeignPayment, IncomingRequest, OutgoingRequest},
    service::PaymentRepository,
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
    incomings: Arc<Mutex<HashMap<PaymentIdentifier, IncomingRequest>>>,
    outgoings: Arc<Mutex<HashMap<PaymentIdentifier, OutgoingRequest>>>,
    foreign: Arc<Mutex<Vec<(PaymentIdentifier, ForeignPayment)>>>,
}

#[async_trait]
impl PaymentRepository for InMemoryPaymentRepo {
    async fn load_incoming(&self, reqid: &PaymentIdentifier) -> Result<IncomingRequest> {
        let locked = self.incomings.lock().expect("load_incoming");
        if let Some(req) = locked.get(reqid) {
            Ok(req.clone())
        } else {
            Err(Error::PaymentRequestNotFound(reqid.clone()))
        }
    }
    async fn store_incoming(&self, req: IncomingRequest) -> Result<()> {
        let mut locked = self.incomings.lock().expect("store_incoming");
        locked.insert(req.reqid.clone(), req);
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

    async fn load_outgoing(&self, reqid: &PaymentIdentifier) -> Result<OutgoingRequest> {
        let locked = self.outgoings.lock().expect("load_outgoing");
        if let Some(req) = locked.get(reqid) {
            Ok(req.clone())
        } else {
            Err(Error::PaymentRequestNotFound(reqid.clone()))
        }
    }

    async fn store_outgoing(&self, req: OutgoingRequest) -> Result<()> {
        let mut locked = self.outgoings.lock().expect("store_outgoing");
        locked.insert(req.reqid.clone(), req);
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

    async fn store_foreign(&self, payment: ForeignPayment) -> Result<()> {
        let mut locked = self.foreign.lock().expect("store_foreign");
        locked.push((payment.reqid.clone(), payment));
        Ok(())
    }
    async fn check_foreign_nonce(&self, nonce: &str) -> Result<Option<ForeignPayment>> {
        let locked = self.foreign.lock().expect("check_foreign_nonce");
        for (_, foreign) in locked.iter() {
            if foreign.nonce == nonce {
                return Ok(Some(foreign.clone()));
            }
        }
        Ok(None)
    }
    async fn check_foreign_reqid(
        &self,
        reqid: &PaymentIdentifier,
    ) -> Result<Option<ForeignPayment>> {
        let locked = self.foreign.lock().expect("check_foreign_reqid");
        for (stored_reqid, foreign) in locked.iter() {
            if *stored_reqid == *reqid {
                return Ok(Some(foreign.clone()));
            }
        }
        Ok(None)
    }
}
