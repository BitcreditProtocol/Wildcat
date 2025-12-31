// ----- standard library imports
use std::ops::Deref;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{core::signature::serialize_n_schnorr_sign_borsh_msg, wire::keys as wire_keys};
use bcr_wdc_webapi::exchange as web_exchange;
use bitcoin::hex::prelude::*;
use cdk::wallet::MintConnector;
use clwdr_client::{model as clwdr_model, ClowderRestClient};
// ----- local imports
use crate::{
    error::{Error, Result},
    foreign::{self, crsat, proof, sat, MintConnectorExt},
};

// ----- end imports

///--------------------------- KeysCl
pub struct CrsatKeysClient {
    keys: bcr_common::client::keys::Client,
}

impl CrsatKeysClient {
    pub fn new(url: reqwest::Url) -> Self {
        let keys = bcr_common::client::keys::Client::new(url);
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
            .post_verify_proofs(*issuer_pk, proofs.clone())
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

        let mint = response
            .mints
            .iter()
            .find(|mint| cashu::PublicKey::from(mint.node_id) == *pk);
        if let Some(mint) = mint {
            return Ok(mint.mint.clone());
        }
        Err(Error::InvalidInput(format!("{pk} not in the alpha set")))
    }

    async fn sign_p2pk_proofs(&self, proofs: &[cashu::Proof]) -> Result<Vec<cashu::Proof>> {
        let response = self.clwdr.post_sign_proofs(proofs).await?;
        Ok(response.proofs)
    }

    async fn can_accept_offline_exchange(
        &self,
        fps: Vec<wire_keys::ProofFingerprint>,
    ) -> Result<(cashu::MintUrl, secp256k1::PublicKey)> {
        let input_amount = fps.iter().fold(cashu::Amount::ZERO, |acc, fp| {
            acc + cashu::Amount::from(fp.amount)
        });
        let fps_len = fps.len();
        let fps: Vec<clwdr_model::ProofFingerprint> = fps.into_iter().collect();
        let clwdr_model::IntermintOriginResponse { node_id, mint_url } =
            self.clwdr.post_fingerprints_origin(fps.clone()).await?;
        let myself = self.clwdr.get_id().await?;
        if node_id != myself.public_key {
            return Err(Error::InvalidInput(String::from(
                "currently not a substitute",
            )));
        }
        let clwdr_model::ValidFingerprints {
            valid_proofs,
            amount,
        } = self.clwdr.post_verify_fingerprints(node_id, fps).await?;
        if valid_proofs.len() != fps_len || amount != input_amount {
            return Err(Error::InvalidInput(String::from(
                "One or more fingerprints are invalid",
            )));
        }
        Ok((mint_url, node_id))
    }

    async fn get_keyset_info(
        &self,
        alpha_pk: &secp256k1::PublicKey,
        kid: &cashu::Id,
    ) -> Result<cashu::KeySetInfo> {
        let cashu::KeysetResponse { mut keysets } =
            self.clwdr.get_keyset_info(alpha_pk, kid).await?;
        if keysets.is_empty() {
            return Err(Error::InvalidInput(String::from(
                "No keyset info found for given kid",
            )));
        }
        Ok(keysets.remove(0))
    }

    async fn get_keyset(
        &self,
        alpha_pk: &secp256k1::PublicKey,
        kid: &cashu::Id,
    ) -> Result<cashu::KeySet> {
        let cashu::KeysResponse { mut keysets } = self.clwdr.get_keyset(alpha_pk, kid).await?;
        if keysets.is_empty() {
            return Err(Error::InvalidInput(String::from(
                "No keyset info found for given kid",
            )));
        }
        Ok(keysets.remove(0))
    }

    async fn is_offline(&self, pk: secp256k1::PublicKey) -> Result<bool> {
        let response = self.clwdr.get_offline(pk).await?;
        Ok(response.offline)
    }
}

pub struct SatKeysClient {
    cl: cdk::wallet::HttpClient,
    signing_keys: bitcoin::secp256k1::Keypair,
}
impl SatKeysClient {
    pub fn new(mint_url: cashu::MintUrl, signing_keys: bitcoin::secp256k1::Keypair) -> Self {
        let cl = cdk::wallet::HttpClient::new(mint_url);
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
        let message = web_exchange::RequestToMintFromForeigneCashPayload {
            foreign_amount_sat: total.into(),
            nonce: nonce.as_hex().to_string(),
        };
        let kp = bitcoin::key::Keypair::new(secp256k1::global::SECP256K1, &mut rand::thread_rng());
        let pk = cashu::PublicKey::from(kp.public_key());
        let (payload, signature) =
            serialize_n_schnorr_sign_borsh_msg(&message, &self.signing_keys)?;
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
        tracing::debug!("minting sat foreign quote id: {}", quote.quote);
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

pub struct MintClientFactory {}
#[async_trait]
impl foreign::MintClientFactory for MintClientFactory {
    async fn make_client(&self, mint_url: cashu::MintUrl) -> Result<Box<dyn MintConnectorExt>> {
        let client = cdk::wallet::HttpClient::new(mint_url);
        Ok(Box::new(client))
    }
}
