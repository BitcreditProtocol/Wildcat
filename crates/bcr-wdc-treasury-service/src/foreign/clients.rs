// ----- standard library imports
use std::{ops::Deref, sync::Arc};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu, cdk,
    client::core::Client as CoreClient,
    wire::{clowder as wire_clowder, keys as wire_keys},
};
use clwdr_client::{model as clwdr_model, ClowderRestClient};
// ----- local imports
use crate::{
    error::{Error, Result},
    foreign::{self, crsat, proof, sat, MintConnectorExt},
};

// ----- end imports

///--------------------------- CrsatCoreClient
pub struct CoreCl {
    pub core: Arc<CoreClient>,
}

#[async_trait]
impl proof::KeysClient for CoreCl {
    async fn sign(&self, blinds: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>> {
        let signatures = self.core.sign(blinds).await?;
        Ok(signatures)
    }
}

#[async_trait]
impl crsat::KeysClient for CoreCl {
    async fn get_keyset_with_expiration(
        &self,
        expiration: chrono::NaiveDate,
    ) -> Result<cashu::KeySet> {
        let kinfo = self
            .core
            .get_or_create_credit_keyset_with_expiration(expiration)
            .await?;
        let keyset = self.core.keys(kinfo.id).await?;
        Ok(keyset)
    }
}

#[async_trait]
impl sat::KeysClient for CoreCl {
    async fn get_active_keyset(&self) -> Result<cashu::KeySet> {
        let filters = wire_keys::KeysetInfoFilters {
            unit: Some(CoreClient::debit_unit()),
            ..Default::default()
        };
        let kinfos = self.core.list_keyset_info(filters).await?;
        let Some(kinfo) = kinfos
            .into_iter()
            .find(|kinfo| kinfo.unit == CoreClient::debit_unit() && kinfo.active)
        else {
            return Err(Error::InvalidInput(String::from(
                "No active keyset found for debit unit",
            )));
        };
        let keyset = self.core.keys(kinfo.id).await?;
        Ok(keyset)
    }
}

///--------------------------- ClowderCl
pub struct ClowderCl {
    pub clwdr: Arc<ClowderRestClient>,
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
        let my_id = self.clwdr.get_info().await?.node_id;
        let pk = bitcoin::PublicKey::from(*my_id);

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
        let fps: Vec<wire_keys::ProofFingerprint> = fps.into_iter().collect();
        let clwdr_model::IntermintOriginResponse {
            node_id: origin_id,
            mint_url: origin_url,
        } = self.clwdr.post_fingerprints_origin(fps.clone()).await?;
        let wire_clowder::ConnectedMintResponse {
            node_id: substitute_id,
            ..
        } = self.clwdr.get_substitute(origin_id).await?;
        let myself = self.clwdr.get_info().await?.node_id;
        if substitute_id != *myself {
            return Err(Error::InvalidInput(String::from(
                "currently not a substitute",
            )));
        }
        let clwdr_model::ValidFingerprints {
            valid_proofs,
            amount,
        } = self.clwdr.post_verify_fingerprints(origin_id, fps).await?;
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
        let response = self.clwdr.get_offline(pk).await?;
        Ok(response.offline)
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
