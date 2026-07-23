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
    async fn store_and_clean(&self, key: Sha1Hash, value: ciborium::Value, now: TStamp) {
        self.store(key, value, now).await;
        self.clean(now).await;
    }
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
    use bcr_common::wire::swap::{
        SignedSwapCommitmentRequest, SwapCommitmentRequest, SwapCommitmentResponse,
    };

    pub fn signed_request_to_key(request: &SignedSwapCommitmentRequest) -> Sha1Hash {
        let mut bytes = Vec::new();
        ciborium::into_writer(request, &mut bytes).unwrap();
        Sha1Hash::hash(&bytes)
    }
    pub fn request_to_key(mut request: SwapCommitmentRequest) -> Sha1Hash {
        request.inputs.inputs.sort_by_key(|input| input.y);
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

pub mod onchain {
    pub mod melt_quote {
        use super::super::*;
        use bcr_common::wire::melt::{MeltQuoteOnchainRequest, MeltQuoteOnchainResponse};

        pub fn request_to_key(mut request: MeltQuoteOnchainRequest) -> Sha1Hash {
            request.inputs.inputs.sort_by_key(|input| input.y);
            let mut bytes = Vec::new();
            ciborium::into_writer(&request, &mut bytes).unwrap();
            Sha1Hash::hash(&bytes)
        }

        pub fn blob_to_response(blob: ciborium::Value) -> MeltQuoteOnchainResponse {
            let mut bytes = Vec::new();
            ciborium::into_writer(&blob, &mut bytes).unwrap();
            ciborium::from_reader(bytes.as_slice()).unwrap()
        }

        pub fn response_to_blob(response: &MeltQuoteOnchainResponse) -> ciborium::Value {
            let mut bytes = Vec::new();
            ciborium::into_writer(response, &mut bytes).unwrap();
            ciborium::from_reader(bytes.as_slice()).unwrap()
        }
    }

    pub mod melt {
        use super::super::*;
        use bcr_common::wire::melt::{MeltOnchainRequest, MeltOnchainResponse};

        pub fn request_to_key(mut request: MeltOnchainRequest) -> Sha1Hash {
            request.inputs.sort_by_key(|input| input.c);
            let mut bytes = Vec::new();
            ciborium::into_writer(&request, &mut bytes).unwrap();
            Sha1Hash::hash(&bytes)
        }

        pub fn blob_to_response(blob: ciborium::Value) -> MeltOnchainResponse {
            let mut bytes = Vec::new();
            ciborium::into_writer(&blob, &mut bytes).unwrap();
            ciborium::from_reader(bytes.as_slice()).unwrap()
        }

        pub fn response_to_blob(response: &MeltOnchainResponse) -> ciborium::Value {
            let mut bytes = Vec::new();
            ciborium::into_writer(response, &mut bytes).unwrap();
            ciborium::from_reader(bytes.as_slice()).unwrap()
        }
    }
}

pub mod ebill {
    pub mod mint {
        use super::super::*;
        use bcr_common::wire::mint as wire_mint;

        pub fn request_to_key(mut request: wire_mint::EbillMintRequest) -> Sha1Hash {
            request.outputs.sort_by_key(|output| output.blinded_secret);
            let mut bytes = Vec::new();
            ciborium::into_writer(&request, &mut bytes).unwrap();
            Sha1Hash::hash(&bytes)
        }

        pub fn blob_to_response(blob: ciborium::Value) -> wire_mint::EbillMintResponse {
            let mut bytes = Vec::new();
            ciborium::into_writer(&blob, &mut bytes).unwrap();
            ciborium::from_reader(bytes.as_slice()).unwrap()
        }

        pub fn response_to_blob(response: &wire_mint::EbillMintResponse) -> ciborium::Value {
            let mut bytes = Vec::new();
            ciborium::into_writer(response, &mut bytes).unwrap();
            ciborium::from_reader(bytes.as_slice()).unwrap()
        }
    }
}
