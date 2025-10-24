// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
// ----- local imports
use crate::{
    commitment,
    error::{Error, Result},
};

// ----- end imports

#[derive(Default)]
pub struct DummySigner {}

#[async_trait]
impl commitment::Signer for DummySigner {
    async fn sign(&self, _content: &[u8]) -> Result<bitcoin::secp256k1::schnorr::Signature> {
        Err(Error::NotYet(
            "DummySigner does not implement signing".to_string(),
        ))
    }
}
