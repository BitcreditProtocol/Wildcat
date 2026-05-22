// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    client::{admin::clowder::Client as ClowderRestClient, core::Client as CoreClient},
    clwdr_client::ClowderNatsClient,
    core::signature,
    wire::{
        attestation::IssuanceAttestation, clowder as wire_clowder, keys as wire_keys,
        melt as wire_melt, mint as wire_mint,
    },
};
use bitcoin::secp256k1::PublicKey;
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    onchain::{ClowderClient, VaultService, WildcatClient},
    vault,
};

// ----- end imports

#[derive(Clone, Debug)]
pub struct WildcatCl {
    pub core_cl: Arc<CoreClient>,
}

#[async_trait]
impl WildcatClient for WildcatCl {
    async fn sign(&self, blinds: Vec<cashu::BlindedMessage>) -> Result<Vec<cashu::BlindSignature>> {
        let signatures = self.core_cl.sign(&blinds).await?;
        Ok(signatures)
    }

    async fn burn(&self, inputs: Vec<cashu::Proof>) -> Result<()> {
        self.core_cl.burn(inputs).await?;
        Ok(())
    }

    async fn recover(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        self.core_cl.recover(proofs).await?;
        Ok(())
    }

    async fn keyset_info(&self, kid: cashu::Id) -> Result<cashu::KeySetInfo> {
        let info = self.core_cl.keyset_info(kid).await?;
        Ok(info)
    }

    async fn keyset(&self, kid: cashu::Id) -> Result<cashu::KeySet> {
        let keyset = self.core_cl.keys(kid).await?;
        Ok(keyset)
    }

    async fn get_active_keyset(&self) -> Result<cashu::Id> {
        let filter = wire_keys::KeysetInfoFilters {
            unit: Some(cashu::CurrencyUnit::Sat),
            ..Default::default()
        };
        let mut infos = self.core_cl.list_keyset_info(filter).await?;
        infos.retain(|info| info.active);
        if infos.is_empty() {
            return Err(Error::Internal(String::from("no active keyset found")));
        }
        infos.sort_by_key(|info| info.final_expiry);
        let last_kid = infos.last().unwrap().id;
        let kid = infos
            .into_iter()
            .find(|info| info.final_expiry.is_none())
            .map(|info| info.id)
            .unwrap_or_else(|| last_kid);
        Ok(kid)
    }

    async fn verify_fingerprints(&self, fps: &[wire_keys::ProofFingerprint]) -> Result<()> {
        for fp in fps {
            self.core_cl.verify_fingerprint(fp).await?;
        }
        Ok(())
    }

    async fn verify_proofs(&self, ps: &[cashu::Proof]) -> Result<()> {
        for p in ps {
            self.core_cl.verify_proof(p).await?;
        }
        Ok(())
    }

    async fn check_spendable(
        &self,
        proofs: Vec<cashu::PublicKey>,
    ) -> Result<Vec<cashu::ProofState>> {
        let states = self.core_cl.check_state(proofs).await?;
        Ok(states)
    }
}

pub struct ClowderCl {
    pub rest: Arc<ClowderRestClient>,
    pub nats: Arc<ClowderNatsClient>,
    pub min_confirmations: u32,
}

#[async_trait]
impl ClowderClient for ClowderCl {
    async fn request_to_pay_bill(
        &self,
        req: wire_clowder::RequestToPayEbillRequest,
        resp: wire_clowder::RequestToPayEbillResponse,
    ) -> Result<()> {
        self.nats.request_to_pay_bill(req, resp).await?;
        Ok(())
    }

    async fn request_onchain_mint_address(
        &self,
        qid: Uuid,
        kid: cashu::Id,
    ) -> Result<bitcoin::Address> {
        let (info, address_response) = futures::try_join!(
            self.rest.get_info(),
            self.rest.request_mint_address(qid, kid)
        )?;
        let address = address_response
            .address
            .require_network(info.network)
            .map_err(|e| Error::Internal(e.to_string()))?;
        Ok(address)
    }

    async fn verify_onchain_mint_payment(
        &self,
        qid: Uuid,
        kid: cashu::Id,
    ) -> Result<bitcoin::Amount> {
        let response = self
            .rest
            .verify_mint_payment(qid, kid, self.min_confirmations)
            .await?;
        Ok(response.amount)
    }

    async fn mint_onchain(
        &self,
        qid: Uuid,
        kid: cashu::Id,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        let output_amount = signatures
            .iter()
            .fold(cashu::Amount::ZERO, |acc, sig| acc + sig.amount);
        let request = wire_clowder::MintOnchainRequest {
            quote_id: qid,
            keyset_id: kid,
            amount: output_amount,
        };
        let response = wire_clowder::MintOnchainResponse { signatures };
        let response = self.nats.mint_onchain(request, response).await?;
        Ok(response.signatures)
    }

    async fn sign_onchain_mint_response(
        &self,
        msg: &wire_mint::OnchainMintQuoteResponseBody,
    ) -> Result<(String, secp256k1::schnorr::Signature)> {
        let request = wire_clowder::MintQuoteOnchainRequest {
            quote_id: msg.quote,
            address: msg.address.clone(),
            payment_amount: msg.payment_amount,
            expiry: msg.expiry,
            blinded_messages: msg.blinded_messages.clone(),
            wallet_key: msg.wallet_key,
        };
        let response = self.nats.mint_quote_onchain(request).await?;
        let content = signature::serialize_borsh_msg_b64(msg)?;
        Ok((content, response.commitment))
    }

    async fn sign_onchain_melt_response(
        &self,
        msg: &wire_melt::MeltQuoteOnchainResponseBody,
        admin_fees: bitcoin::Amount,
        network_fees: bitcoin::Amount,
    ) -> Result<(String, secp256k1::schnorr::Signature)> {
        let request = wire_clowder::MeltQuoteOnchainRequest {
            quote_id: msg.quote,
            inputs: msg.inputs.clone(),
            address: msg.address.clone(),
            admin_fees: cashu::Amount::from(admin_fees.to_sat()),
            network_fees,
            expiry: msg.expiry,
            wallet_key: msg.wallet_key,
        };
        let response = self.nats.melt_quote_onchain(request).await?;
        let content = signature::serialize_borsh_msg_b64(msg)?;
        Ok((content, response.commitment))
    }

    async fn verify_onchain_address(
        &self,
        address: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    ) -> Result<bitcoin::Address> {
        let info = self.rest.get_info().await?;
        let address = address.require_network(info.network)?;
        Ok(address)
    }

    async fn melt_onchain(
        &self,
        qid: Uuid,
        amount: bitcoin::Amount,
        address: bitcoin::Address,
        inputs: Vec<cashu::Proof>,
        fees: Vec<cashu::BlindSignature>,
        commitment: secp256k1::schnorr::Signature,
        attestation: IssuanceAttestation,
    ) -> Result<wire_melt::MeltTx> {
        let request = wire_clowder::MeltOnchainRequest {
            quote: qid,
            address: address.into_unchecked(),
            amount,
            inputs,
            commitment,
            fees,
            attestation,
        };
        let response = self.nats.melt_onchain(request).await?;
        Ok(response.txid)
    }

    async fn fetch_mint_signatures(
        &self,
        qid: Uuid,
        mint_id: secp256k1::PublicKey,
    ) -> Result<Vec<cashu::BlindSignature>> {
        let response = self
            .rest
            .fetch_mint_onchain_signatures(&mint_id, &qid)
            .await?
            .ok_or(Error::ResourceNotFound(format!(
                "on chain mint {qid} in {mint_id} not found"
            )))?;
        Ok(response)
    }

    async fn estimate_onchain_fees(&self, _amount: bitcoin::Amount) -> Result<bitcoin::Amount> {
        tracing::error!("unimplemented, returning default fee rate for 2 onchain transactions");
        Ok(bitcoin::Amount::from_sat(1000))
    }

    async fn get_onchain_reserve(&self) -> Result<bitcoin::Amount> {
        let collaterals = self.rest.get_mint_collateral().await?;
        Ok(collaterals.onchain)
    }

    async fn verify_attestation(
        &self,
        alpha_id: &PublicKey,
        inputs: &[cashu::Proof],
        attestation: &IssuanceAttestation,
    ) -> Result<()> {
        bcr_wdc_utils::attestation::verify(&self.rest, alpha_id, inputs, attestation).await?;
        Ok(())
    }
}

pub struct VaultSrvc {
    pub vault: Arc<vault::Service>,
}

#[async_trait]
impl VaultService for VaultSrvc {
    async fn store_proofs(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        self.vault.store_proofs(proofs).await?;
        Ok(())
    }
}
