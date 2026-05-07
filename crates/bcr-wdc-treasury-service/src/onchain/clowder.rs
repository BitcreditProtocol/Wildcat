// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    client::admin::clowder::Client as ClowderRestClient,
    clwdr_client::ClowderNatsClient,
    core::signature,
    wire::{
        attestation::{self as wire_attestation, IssuanceAttestation},
        clowder::messages as clowder_messages,
        melt as wire_melt,
        mint as wire_mint,
    },
};
use bitcoin::secp256k1::PublicKey;
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    onchain::ClowderClient,
};

// ----- end imports

pub struct ClowderCl {
    pub rest: Arc<ClowderRestClient>,
    pub nats: Arc<ClowderNatsClient>,
    pub min_confirmations: u32,
}

#[async_trait]
impl ClowderClient for ClowderCl {
    async fn get_sweep(&self, qid: uuid::Uuid) -> Result<bitcoin::Address> {
        let dummy_kid = cashu::Id::from_bytes(&[0_u8; 8])
            .map_err(|_| crate::error::Error::InvalidInput(String::from("Invalid keyset ID")))?;
        let response = self.rest.request_mint_address(qid, dummy_kid).await?;
        Ok(response.address.assume_checked())
    }

    async fn request_to_pay_bill(
        &self,
        req: clowder_messages::RequestToPayEbillRequest,
        resp: clowder_messages::RequestToPayEbillResponse,
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
        let request = clowder_messages::MintOnchainRequest {
            quote_id: qid,
            keyset_id: kid,
            amount: output_amount,
        };
        let response = clowder_messages::MintOnchainResponse { signatures };
        let response = self.nats.mint_onchain(request, response).await?;
        Ok(response.signatures)
    }

    async fn sign_onchain_mint_response(
        &self,
        msg: &wire_mint::OnchainMintQuoteResponseBody,
    ) -> Result<(String, secp256k1::schnorr::Signature)> {
        let request = clowder_messages::MintQuoteOnchainRequest {
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
    ) -> Result<(String, secp256k1::schnorr::Signature)> {
        let request = clowder_messages::MeltQuoteOnchainRequest {
            quote_id: msg.quote,
            inputs: msg.inputs.clone(),
            address: msg.address.clone(),
            amount: msg.amount,
            total: msg.total,
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
        commitment: secp256k1::schnorr::Signature,
        attestation: IssuanceAttestation,
    ) -> Result<wire_melt::MeltTx> {
        let request = clowder_messages::MeltOnchainRequest {
            quote: qid,
            address: address.into_unchecked(),
            amount,
            inputs,
            commitment,
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

    async fn verify_attestation(
        &self,
        alpha_id: &PublicKey,
        inputs: &[cashu::Proof],
        attestation: &IssuanceAttestation,
    ) -> Result<()> {
        let betas = self.rest.get_betas().await?;
        let beta = betas
            .mints
            .iter()
            .find(|b| b.node_id == attestation.beta_id)
            .ok_or(Error::Attestation(
                wire_attestation::AttestationError::UnknownBeta(attestation.beta_id),
            ))?;
        wire_attestation::verify_attestation_local(alpha_id, inputs, attestation, |id| {
            id == &attestation.beta_id
        })
        .map_err(Error::Attestation)?;
        let beta_cl = ClowderRestClient::new(beta.clowder.clone());
        let response = beta_cl
            .post_attest_verify(&wire_attestation::AttestationVerifyRequest {
                alpha_id: *alpha_id,
                attestation: attestation.clone(),
            })
            .await?;
        wire_attestation::verify_attestation_response(
            alpha_id,
            &attestation.beta_id,
            attestation,
            &response,
        )
        .map_err(Error::Attestation)?;
        Ok(())
    }
}
