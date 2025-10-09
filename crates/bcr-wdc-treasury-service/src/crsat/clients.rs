// ----- standard library imports
use std::ops::Deref;
// ----- extra library imports
use async_trait::async_trait;
use clwdr_client::ClowderRestClient;
// ----- local imports
use crate::{
    crsat,
    error::{Error, Result},
};

// ----- end imports

///--------------------------- KeysCl
pub struct KeysCl {
    keys: bcr_common::KeysClient,
}

impl KeysCl {
    pub fn new(url: reqwest::Url) -> Self {
        let keys = bcr_common::KeysClient::new(url);
        Self { keys }
    }
}

#[async_trait]
impl crsat::proof::KeysClient for KeysCl {
    async fn sign(&self, blinds: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>> {
        let mut signatures: Vec<cashu::BlindSignature> = Vec::with_capacity(blinds.len());
        for b in blinds {
            let sig = self.keys.sign(b).await?;
            signatures.push(sig);
        }
        Ok(signatures)
    }
}

#[async_trait]
impl crsat::KeysClient for KeysCl {
    async fn get_keyset_with_expiration(
        &self,
        expiration: chrono::NaiveDate,
    ) -> Result<cashu::KeySet> {
        let kid = self.keys.keys_for_expiration(expiration).await?;
        let keyset = self.keys.keys(kid).await?;
        Ok(keyset)
    }
}

///--------------------------- ClowderCl
pub struct ClowderCl {
    clwdr: ClowderRestClient,
}

impl ClowderCl {
    pub fn new(url: reqwest::Url) -> Self {
        let clwdr = ClowderRestClient::new(url);
        Self { clwdr }
    }
}

#[async_trait]
impl crsat::proof::ClowderClient for ClowderCl {
    async fn check_htlc_proofs(
        &self,
        issuer: cashu::PublicKey,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let issuer_pk: &bitcoin::secp256k1::PublicKey = issuer.deref();
        let response = self
            .clwdr
            .post_verify_intermint_proofs(*issuer_pk, proofs.clone())
            .await?;
        if response.valid_proofs != proofs {
            return Err(Error::InvalidInput(String::from(
                "One or more proofs are invalid",
            )));
        }
        let response = self.clwdr.post_validate_wallet_lock(&proofs).await?;
        if !response.success {
            return Err(Error::InvalidInput(String::from(
                "One or more proofs failed wallet lock validation",
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl crsat::ClowderClient for ClowderCl {
    async fn get_myself_pk(&self) -> Result<bitcoin::PublicKey> {
        let response = self.clwdr.get_id().await?;
        let pk = bitcoin::PublicKey::from(response.public_key);
        Ok(pk)
    }

    async fn get_mint_url_from_pk(&self, pk: &cashu::PublicKey) -> Result<cashu::MintUrl> {
        let response = self.clwdr.get_alphas().await?;
        for idx in 0..response.node_ids.len() {
            let alpha_pk = cashu::PublicKey::from(response.node_ids[idx]);
            if alpha_pk == *pk {
                return Ok(response.mint_urls[idx].clone());
            }
        }
        Err(Error::InvalidInput(format!("{pk} not in the alpha set")))
    }
    async fn sign_p2pk_proofs(&self, proofs: &[cashu::Proof]) -> Result<Vec<cashu::Proof>> {
        let response = self.clwdr.post_sign_proofs(proofs).await?;
        Ok(response.proofs)
    }
}
