// ----- standard library imports
use std::sync::{Arc, Mutex};
// ----- extra library imports
use async_trait::async_trait;
// ----- local imports
use crate::error::Result;
use crate::onchain::{PrivateKeysRepository, SingleKeyWallet};

// ----- end imports

#[derive(Default, Debug, Clone)]
pub struct InMemoryKeys {
    keys: Arc<Mutex<Vec<SingleKeyWallet>>>,
}

#[async_trait]
impl PrivateKeysRepository for InMemoryKeys {
    async fn get_private_keys(&self) -> Result<Vec<SingleKeyWallet>> {
        let locked = self.keys.lock().expect("get_private_keys");
        Ok(locked.clone())
    }

    async fn add_key(&self, key: SingleKeyWallet) -> Result<()> {
        let mut locked = self.keys.lock().expect("add_key");
        locked.push(key);
        Ok(())
    }
}
