// ----- standard library imports
use std::{ops::Deref, sync::Arc};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu, cdk,
    client::{admin::clowder::Client as ClowderClient, core::Client as CoreClient},
    wire::{
        clowder::{self as wire_clowder, messages as clwdr_msgs},
        keys as wire_keys,
    },
};
use bitcoin::secp256k1;
// ----- local imports
use crate::{
    error::{Error, Result},
    foreign::{self, MintConnectorExt},
};

// ----- end imports

///--------------------------- CrsatCoreClient
pub struct CoreCl {
    pub core: Arc<CoreClient>,
}

#[async_trait]
impl foreign::KeysClient for CoreCl {
    async fn get_keyset_with_expiration(
        &self,
        expiration: chrono::NaiveDate,
    ) -> Result<cashu::KeySet> {
        let kinfo = self
            .core
            .get_or_create_keyset_with_expiration(expiration)
            .await?;
        let keyset = self.core.keys(kinfo.id).await?;
        Ok(keyset)
    }
    async fn sign(&self, blinds: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>> {
        let signatures = self.core.sign(blinds).await?;
        Ok(signatures)
    }
}

///--------------------------- ClowderCl
pub struct ClowderCl {
    pub clwdr: Arc<ClowderClient>,
}

#[async_trait]
impl foreign::ClowderClient for ClowderCl {
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

    async fn get_myself_pk(&self) -> Result<secp256k1::PublicKey> {
        let my_cashu_pk = self.clwdr.get_info().await?.node_id;
        let my_id = secp256k1::PublicKey::from_slice(&my_cashu_pk.to_bytes())
            .expect("secp256k1::PublicKey == cashu::PublicKey");
        Ok(my_id)
    }

    async fn get_mint_url_from_pk(&self, pk: &secp256k1::PublicKey) -> Result<reqwest::Url> {
        let response = self.clwdr.get_mint_url(pk).await?;
        Ok(response.mint_url)
    }

    async fn sign_p2pk_proofs(&self, proofs: &[cashu::Proof]) -> Result<Vec<cashu::Proof>> {
        let response = self.clwdr.post_sign_proofs(proofs).await?;
        Ok(response.proofs)
    }

    async fn can_accept_offline_exchange(
        &self,
        fps: Vec<wire_keys::ProofFingerprint>,
    ) -> Result<(reqwest::Url, secp256k1::PublicKey)> {
        let input_amount = fps.iter().fold(cashu::Amount::ZERO, |acc, fp| {
            acc + cashu::Amount::from(fp.amount)
        });
        let fps_len = fps.len();
        let fps: Vec<wire_keys::ProofFingerprint> = fps.into_iter().collect();
        let clwdr_msgs::IntermintOriginResponse {
            node_id: origin_id,
            mint_url: origin_url,
        } = self.clwdr.post_fingerprints_origin(fps.clone()).await?;
        let wire_clowder::ConnectedMintResponse {
            node_id: substitute_id,
            ..
        } = self.clwdr.get_substitute(&origin_id).await?;
        let myself = self.clwdr.get_info().await?.node_id;
        if substitute_id != *myself {
            return Err(Error::InvalidInput(String::from(
                "currently not a substitute",
            )));
        }
        let clwdr_msgs::ValidFingerprints {
            valid_proofs,
            amount,
        } = self.clwdr.post_verify_fingerprints(&origin_id, fps).await?;
        if valid_proofs.len() != fps_len || amount != input_amount {
            return Err(Error::InvalidInput(String::from(
                "One or more fingerprints are invalid",
            )));
        }
        Ok((origin_url, origin_id))
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
        let response = self.clwdr.get_offline(&pk).await?;
        Ok(response.offline)
    }
}

pub struct CdkMintClient(cdk::wallet::HttpClient);

impl std::fmt::Debug for CdkMintClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CdkMintClient").finish()
    }
}

#[async_trait]
impl cdk::wallet::MintConnector for CdkMintClient {
    async fn get_mint_keys(&self) -> std::result::Result<Vec<cashu::KeySet>, cdk::Error> {
        self.0.get_mint_keys().await
    }
    async fn get_mint_keyset(
        &self,
        keyset_id: cashu::Id,
    ) -> std::result::Result<cashu::KeySet, cdk::Error> {
        self.0.get_mint_keyset(keyset_id).await
    }
    async fn get_mint_keysets(&self) -> std::result::Result<cashu::KeysetResponse, cdk::Error> {
        self.0.get_mint_keysets().await
    }
    async fn post_mint_quote(
        &self,
        request: cashu::MintQuoteBolt11Request,
    ) -> std::result::Result<cashu::MintQuoteBolt11Response<String>, cdk::Error> {
        self.0.post_mint_quote(request).await
    }
    async fn get_mint_quote_status(
        &self,
        quote_id: &str,
    ) -> std::result::Result<cashu::MintQuoteBolt11Response<String>, cdk::Error> {
        self.0.get_mint_quote_status(quote_id).await
    }
    async fn post_mint(
        &self,
        request: cashu::MintRequest<String>,
    ) -> std::result::Result<cashu::MintResponse, cdk::Error> {
        self.0.post_mint(request).await
    }
    async fn post_melt_quote(
        &self,
        request: cashu::MeltQuoteBolt11Request,
    ) -> std::result::Result<cashu::MeltQuoteBolt11Response<String>, cdk::Error> {
        self.0.post_melt_quote(request).await
    }
    async fn get_melt_quote_status(
        &self,
        quote_id: &str,
    ) -> std::result::Result<cashu::MeltQuoteBolt11Response<String>, cdk::Error> {
        self.0.get_melt_quote_status(quote_id).await
    }
    async fn post_melt(
        &self,
        request: cashu::MeltRequest<String>,
    ) -> std::result::Result<cashu::MeltQuoteBolt11Response<String>, cdk::Error> {
        self.0.post_melt(request).await
    }
    async fn post_swap(
        &self,
        request: cashu::SwapRequest,
    ) -> std::result::Result<cashu::SwapResponse, cdk::Error> {
        self.0.post_swap(request).await
    }
    async fn get_mint_info(&self) -> std::result::Result<cashu::MintInfo, cdk::Error> {
        self.0.get_mint_info().await
    }
    async fn post_check_state(
        &self,
        request: cashu::CheckStateRequest,
    ) -> std::result::Result<cashu::CheckStateResponse, cdk::Error> {
        self.0.post_check_state(request).await
    }
    async fn post_restore(
        &self,
        request: cashu::RestoreRequest,
    ) -> std::result::Result<cashu::RestoreResponse, cdk::Error> {
        self.0.post_restore(request).await
    }
    async fn post_mint_bolt12_quote(
        &self,
        request: cashu::MintQuoteBolt12Request,
    ) -> std::result::Result<cashu::MintQuoteBolt12Response<String>, cdk::Error> {
        self.0.post_mint_bolt12_quote(request).await
    }
    async fn get_mint_quote_bolt12_status(
        &self,
        quote_id: &str,
    ) -> std::result::Result<cashu::MintQuoteBolt12Response<String>, cdk::Error> {
        self.0.get_mint_quote_bolt12_status(quote_id).await
    }
    async fn post_melt_bolt12_quote(
        &self,
        request: cashu::MeltQuoteBolt12Request,
    ) -> std::result::Result<cashu::MeltQuoteBolt11Response<String>, cdk::Error> {
        self.0.post_melt_bolt12_quote(request).await
    }
    async fn get_melt_bolt12_quote_status(
        &self,
        quote_id: &str,
    ) -> std::result::Result<cashu::MeltQuoteBolt11Response<String>, cdk::Error> {
        self.0.get_melt_bolt12_quote_status(quote_id).await
    }
    async fn post_melt_bolt12(
        &self,
        request: cashu::MeltRequest<String>,
    ) -> std::result::Result<cashu::MeltQuoteBolt11Response<String>, cdk::Error> {
        self.0.post_melt_bolt12(request).await
    }
}

#[async_trait]
impl MintConnectorExt for CdkMintClient {
    async fn swap(
        &self,
        request: bcr_common::wire::swap::SwapRequest,
    ) -> std::result::Result<bcr_common::wire::swap::SwapResponse, cdk::Error> {
        let cashu_request = cashu::SwapRequest::new(request.inputs, request.outputs);
        let cashu_response =
            <Self as cdk::wallet::MintConnector>::post_swap(self, cashu_request).await?;
        Ok(bcr_common::wire::swap::SwapResponse {
            signatures: cashu_response.signatures,
        })
    }
}

pub struct MintClientFactory {}
#[async_trait]
impl foreign::MintClientFactory for MintClientFactory {
    async fn make_client(&self, mint_url: cashu::MintUrl) -> Result<Box<dyn MintConnectorExt>> {
        let client = CdkMintClient(cdk::wallet::HttpClient::new(mint_url));
        Ok(Box::new(client))
    }
}
