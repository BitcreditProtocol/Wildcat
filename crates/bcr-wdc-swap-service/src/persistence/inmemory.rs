// ----- standard library imports
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
// ----- extra library imports
use async_trait::async_trait;
use cashu::dhke as cdk_dhke;
use cashu::{nut00 as cdk00, nut01 as cdk01, nut07 as cdk07};
// ----- local imports
use crate::error::{Error, Result};
use crate::service::ProofRepository;

#[allow(dead_code)]
#[derive(Default, Clone)]
pub struct ProofMap {
    proofs: Arc<Mutex<HashMap<cdk01::PublicKey, cdk00::Proof>>>,
}

#[async_trait()]
impl ProofRepository for ProofMap {
    async fn insert(&self, tokens: &[cdk00::Proof]) -> Result<()> {
        let mut items = Vec::with_capacity(tokens.len());
        for token in tokens {
            let y = cdk_dhke::hash_to_curve(&token.secret.to_bytes()).map_err(Error::CdkDhke)?;
            items.push((y, token.clone()));
        }
        let mut locked = self.proofs.lock().unwrap();
        for (y, _) in &items {
            if locked.contains_key(y) {
                return Err(Error::ProofsAlreadySpent);
            }
        }
        for (y, token) in items.into_iter() {
            locked.insert(y, token);
        }
        Ok(())
    }
    async fn remove(&self, tokens: &[cdk00::Proof]) -> Result<()> {
        let mut locked = self.proofs.lock().unwrap();
        for token in tokens {
            let y = cdk_dhke::hash_to_curve(&token.secret.to_bytes()).map_err(Error::CdkDhke)?;
            locked.remove(&y);
        }
        Ok(())
    }

    async fn contains(&self, y: cashu::PublicKey) -> Result<Option<cdk07::ProofState>> {
        let locked = self.proofs.lock().unwrap();
        if locked.get(&y).is_some() {
            let ret_v = cdk07::ProofState {
                y,
                state: cdk07::State::Spent,
                witness: None,
            };
            return Ok(Some(ret_v));
        }
        Ok(None)
    }
}
