// ----- standard library imports
use std::ops::Deref;
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_webapi::exchange as web_exchange;
use bitcoin::hex::prelude::*;
use cdk::wallet::MintConnector;
use clwdr_client::ClowderRestClient;
// ----- local imports
use crate::{
    error::{Error, Result},
    foreign::{self, crsat, proof, sat},
};

// ----- end imports

///--------------------------- KeysCl
pub struct CrsatKeysClient {
    keys: bcr_common::KeysClient,
}

impl CrsatKeysClient {
    pub fn new(url: reqwest::Url) -> Self {
        let keys = bcr_common::KeysClient::new(url);
        Self { keys }
    }
}

#[async_trait]
impl proof::KeysClient for CrsatKeysClient {
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
impl crsat::KeysClient for CrsatKeysClient {
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
impl proof::ClowderClient for ClowderCl {
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
impl foreign::ClowderClient for ClowderCl {
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

pub struct SatKeysClient {
    cl: cdk::wallet::HttpClient,
    signing_keys: bitcoin::secp256k1::Keypair,
}
impl SatKeysClient {
    pub fn new(mint_url: cashu::MintUrl, signing_keys: bitcoin::secp256k1::Keypair) -> Self {
        let cl = cdk::wallet::HttpClient::new(mint_url, None);
        Self { cl, signing_keys }
    }
}

#[async_trait]
impl proof::KeysClient for SatKeysClient {
    async fn sign(&self, blinds: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>> {
        let total = blinds
            .iter()
            .fold(cashu::Amount::ZERO, |acc, b| acc + b.amount);
        let nonce: [u8; 16] = rand::random();
        let message = web_exchange::RequestToMintFromForeignCashPayload {
            foreign_amount_sat: total.into(),
            nonce: nonce.as_hex().to_string(),
        };
        let kp = bitcoin::key::Keypair::new(secp256k1::global::SECP256K1, &mut rand::thread_rng());
        let pk = cashu::PublicKey::from(kp.public_key());
        let payload = bitcoin::base64::encode(borsh::to_vec(&message)?);
        let signature = bcr_common::core::signature::sign_with_key(&message, &self.signing_keys)?;
        let description = serde_json::to_string(&web_exchange::RequestToMintFromForeigneCash {
            payload,
            signature,
        })?;
        let request = cashu::MintQuoteBolt11Request {
            amount: total,
            unit: cashu::CurrencyUnit::Sat,
            description: Some(description),
            pubkey: Some(pk),
        };
        let quote = self.cl.post_mint_quote(request).await?;
        let mut request = cashu::MintRequest {
            quote: quote.quote,
            outputs: blinds.to_vec(),
            signature: None,
        };
        request.sign(kp.secret_key().into())?;
        let response = self.cl.post_mint(request).await?;
        Ok(response.signatures)
    }
}

#[async_trait]
impl sat::KeysClient for SatKeysClient {
    async fn get_active_keyset(&self) -> Result<cashu::KeySet> {
        let keyset_infos = self.cl.get_mint_keysets().await?;
        for info in keyset_infos.keysets {
            if info.active {
                let keyset = self.cl.get_mint_keyset(info.id).await?;
                return Ok(keyset);
            }
        }
        Err(Error::InvalidInput(String::from(
            "No active keyset found on mint",
        )))
    }
}
