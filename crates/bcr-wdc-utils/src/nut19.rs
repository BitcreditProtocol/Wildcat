// ----- standard library imports
use std::collections::HashMap;
// ----- extra library imports
use async_trait::async_trait;
use bitcoin::hashes::{sha1::Hash as Sha1Hash, Hash};
use tokio::sync::RwLock;
// ----- local imports
use crate::TStamp;

// ----- end imports

#[async_trait]
pub trait Cache: Send + Sync {
    async fn store(&self, key: Sha1Hash, value: ciborium::Value, now: TStamp);
    async fn load(&self, key: Sha1Hash) -> Option<ciborium::Value>;
    async fn clean(&self, now: TStamp);
}

#[derive(Debug)]
pub struct InMemoryMap {
    map: RwLock<HashMap<Sha1Hash, (TStamp, ciborium::Value)>>,
    oldest: RwLock<TStamp>,
    ttl: chrono::Duration,
}

impl InMemoryMap {
    pub fn new(ttl: chrono::Duration) -> Self {
        Self {
            map: RwLock::new(HashMap::new()),
            oldest: RwLock::new(TStamp::MAX_UTC),
            ttl,
        }
    }
}

#[async_trait]
impl Cache for InMemoryMap {
    async fn store(&self, key: Sha1Hash, value: ciborium::Value, now: TStamp) {
        let mut map = self.map.write().await;
        map.insert(key, (now, value));
    }

    async fn load(&self, key: Sha1Hash) -> Option<ciborium::Value> {
        let map = self.map.read().await;
        map.get(&key).cloned().map(|(_, value)| value)
    }

    async fn clean(&self, now: TStamp) {
        let mut oldest = self.oldest.write().await;
        if now - *oldest < self.ttl {
            return;
        }
        let mut map = self.map.write().await;
        let mut new_oldest = TStamp::MAX_UTC;
        map.retain(|_, (tstamp, _)| {
            if now - *tstamp > self.ttl {
                false
            } else if *tstamp < new_oldest {
                new_oldest = *tstamp;
                true
            } else {
                true
            }
        });
        *oldest = new_oldest;
    }
}

pub struct Dummy;
#[async_trait]
impl Cache for Dummy {
    async fn store(&self, _key: Sha1Hash, _value: ciborium::Value, _now: TStamp) {}
    async fn load(&self, _key: Sha1Hash) -> Option<ciborium::Value> {
        None
    }
    async fn clean(&self, _now: TStamp) {}
}

pub mod swap_commitment {
    use super::*;
    use bcr_common::wire::swap::{SwapCommitmentRequest, SwapCommitmentResponse};

    pub fn request_to_key(mut request: SwapCommitmentRequest) -> Sha1Hash {
        request.inputs.sort_by_key(|input| input.y);
        request.outputs.sort_by_key(|input| input.blinded_secret);
        let mut bytes = Vec::new();
        ciborium::into_writer(&request, &mut bytes).unwrap();
        Sha1Hash::hash(&bytes)
    }

    pub fn blob_to_response(blob: ciborium::Value) -> SwapCommitmentResponse {
        let mut bytes = Vec::new();
        ciborium::into_writer(&blob, &mut bytes).unwrap();
        ciborium::from_reader(bytes.as_slice()).unwrap()
    }

    pub fn response_to_blob(response: &SwapCommitmentResponse) -> ciborium::Value {
        let mut bytes = Vec::new();
        ciborium::into_writer(response, &mut bytes).unwrap();
        ciborium::from_reader(bytes.as_slice()).unwrap()
    }
}

pub mod swap {
    use super::*;
    use bcr_common::wire::swap::{SwapRequest, SwapResponse};

    pub fn request_to_key(mut request: SwapRequest) -> Sha1Hash {
        request.inputs.sort_by_key(|input| input.c);
        request.outputs.sort_by_key(|input| input.blinded_secret);
        let mut bytes = Vec::new();
        ciborium::into_writer(&request, &mut bytes).unwrap();
        Sha1Hash::hash(&bytes)
    }

    pub fn blob_to_response(blob: ciborium::Value) -> SwapResponse {
        let mut bytes = Vec::new();
        ciborium::into_writer(&blob, &mut bytes).unwrap();
        ciborium::from_reader(bytes.as_slice()).unwrap()
    }

    pub fn response_to_blob(response: &SwapResponse) -> ciborium::Value {
        let mut bytes = Vec::new();
        ciborium::into_writer(response, &mut bytes).unwrap();
        ciborium::from_reader(bytes.as_slice()).unwrap()
    }
}

pub mod signed_swap {
    use super::*;
    use bcr_common::wire::swap::{SignedSwapRequest, SwapResponse};

    pub fn request_to_key(request: &SignedSwapRequest) -> Sha1Hash {
        let mut bytes = Vec::new();
        ciborium::into_writer(request, &mut bytes).unwrap();
        Sha1Hash::hash(&bytes)
    }

    pub fn blob_to_response(blob: ciborium::Value) -> SwapResponse {
        let mut bytes = Vec::new();
        ciborium::into_writer(&blob, &mut bytes).unwrap();
        ciborium::from_reader(bytes.as_slice()).unwrap()
    }

    pub fn response_to_blob(response: &SwapResponse) -> ciborium::Value {
        let mut bytes = Vec::new();
        ciborium::into_writer(response, &mut bytes).unwrap();
        ciborium::from_reader(bytes.as_slice()).unwrap()
    }
}
