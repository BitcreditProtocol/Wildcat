// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    client::{admin::clowder::Client as ClowderClient, core::Client as CoreClient},
    clwdr_client::ClowderNatsClient,
    wire::{clowder as wire_clowder, keys as wire_keys},
};
// ----- local imports
use crate::{
    error::{Error, Result},
    foreign::{self, ForeignClient},
    TStamp,
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
    pub rest: Arc<ClowderClient>,
    pub stream: Arc<ClowderNatsClient>,
}

#[async_trait]
impl foreign::ClowderClient for ClowderCl {
    async fn check_htlc_proofs(
        &self,
        issuer: secp256k1::PublicKey,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let response = self.rest.post_verify_proofs(issuer, proofs.clone()).await?;
        if response.valid_proofs != proofs {
            return Err(Error::InvalidInput(String::from(
                "One or more proofs are invalid",
            )));
        }
        let response = self.rest.post_validate_wallet_lock(&proofs).await?;
        if !response.success {
            return Err(Error::InvalidInput(String::from(
                "One or more proofs failed wallet lock validation",
            )));
        }
        Ok(())
    }

    async fn get_myself_pk(&self) -> Result<secp256k1::PublicKey> {
        let my_cashu_pk = self.rest.get_info().await?.node_id;
        let my_id = secp256k1::PublicKey::from_slice(&my_cashu_pk.to_bytes())
            .expect("secp256k1::PublicKey == cashu::PublicKey");
        Ok(my_id)
    }

    async fn get_mint_url_from_pk(&self, pk: &secp256k1::PublicKey) -> Result<reqwest::Url> {
        let response = self.rest.get_mint_url(pk).await?;
        Ok(response.mint_url)
    }

    async fn sign_p2pk_proofs(&self, proofs: &[cashu::Proof]) -> Result<Vec<cashu::Proof>> {
        let response = self.rest.post_sign_proofs(proofs).await?;
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
        let wire_clowder::IntermintOriginResponse {
            node_id: origin_id,
            mint_url: origin_url,
        } = self.rest.post_fingerprints_origin(fps.clone()).await?;
        let wire_clowder::ConnectedMintResponse {
            node_id: substitute_id,
            ..
        } = self.rest.get_substitute(&origin_id).await?;
        let myself = self.rest.get_info().await?.node_id;
        if substitute_id != *myself {
            return Err(Error::InvalidInput(String::from(
                "currently not a substitute",
            )));
        }
        let wire_clowder::ValidFingerprints {
            valid_proofs,
            amount,
        } = self.rest.post_verify_fingerprints(&origin_id, fps).await?;
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
            self.rest.get_keyset_info(alpha_pk, kid).await?;
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
        let cashu::KeysResponse { mut keysets } = self.rest.get_keyset(alpha_pk, kid).await?;
        if keysets.is_empty() {
            return Err(Error::InvalidInput(String::from(
                "No keyset info found for given kid",
            )));
        }
        Ok(keysets.remove(0))
    }

    async fn is_offline(&self, pk: secp256k1::PublicKey) -> Result<bool> {
        let response = self.rest.get_offline(&pk).await?;
        Ok(response.offline)
    }

    async fn signal_online_exchange_event(
        &self,
        inputs: Vec<cashu::Proof>,
        outputs: Vec<cashu::Proof>,
        path: Vec<secp256k1::PublicKey>,
    ) -> Result<Vec<cashu::Proof>> {
        let request = wire_clowder::MintForeignEcashRequest {
            proofs: inputs,
            exchange_path: path,
        };
        let response = wire_clowder::MintForeignEcashResponse { proofs: outputs };
        let wire_clowder::MintForeignEcashResponse { proofs } =
            self.stream.mint_foreign_ecash(request, response).await?;
        Ok(proofs)
    }
}

pub struct MintClient {
    cl: bcr_common::client::mint::Client,
    my_pk: secp256k1::PublicKey,
    foreign_pk: secp256k1::PublicKey,
}

#[async_trait]
impl ForeignClient for MintClient {
    async fn swap(
        &self,
        inputs: Vec<cashu::Proof>,
        outputs: Vec<cashu::BlindedMessage>,
        now: TStamp,
    ) -> Result<Vec<cashu::BlindSignature>> {
        let fps = inputs
            .iter()
            .cloned()
            .map(wire_keys::ProofFingerprint::try_from)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let expiry = now + chrono::Duration::minutes(1);
        let (_, commitment) = self
            .cl
            .commit_swap(
                fps,
                outputs.clone(),
                expiry.timestamp() as u64,
                self.my_pk,
                self.foreign_pk,
            )
            .await?;
        let signatures = self.cl.swap(inputs, outputs, commitment).await?;
        Ok(signatures)
    }

    async fn check_state(&self, ys: Vec<cashu::PublicKey>) -> Result<Vec<cashu::ProofState>> {
        let states = self.cl.check_state(ys).await?;
        Ok(states)
    }

    async fn get_keyset(&self, kid: cashu::Id) -> Result<cashu::KeySet> {
        let keys = self.cl.keys(kid).await?;
        Ok(keys)
    }
}

pub struct MintClientFactory {
    pub my_pk: secp256k1::PublicKey,
}
#[async_trait]
impl foreign::MintClientFactory for MintClientFactory {
    async fn make_client(
        &self,
        mint_url: reqwest::Url,
        mint_pk: secp256k1::PublicKey,
    ) -> Result<Box<dyn ForeignClient>> {
        let cl = bcr_common::client::mint::Client::new(mint_url);
        Ok(Box::new(MintClient {
            cl,
            my_pk: self.my_pk,
            foreign_pk: mint_pk,
        }))
    }
}
