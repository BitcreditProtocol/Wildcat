// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    wire::{clowder::messages as clowder_messages, melt as wire_melt, mint as wire_mint},
};
use bitcoin::{base64::prelude::*, hashes::Hash};
use clwdr_client::{ClowderNatsClient, ClowderRestClient, SignatoryNatsClient};
use uuid::Uuid;
// ----- local imports
use crate::{
    debit::ClowderClient,
    error::{Error, Result},
};

// ----- end imports

pub struct ClowderCl {
    pub rest: Arc<ClowderRestClient>,
    pub nats: Arc<ClowderNatsClient>,
    pub signatory: Arc<SignatoryNatsClient>,
    pub min_confirmations: u32,
}

#[async_trait]
impl ClowderClient for ClowderCl {
    async fn get_sweep(&self, qid: uuid::Uuid) -> Result<bitcoin::Address> {
        let dummy_kid = cashu::Id::from_bytes(&[0_u8; 8])
            .map_err(|_| crate::error::Error::InvalidInput(String::from("Invalid keyset ID")))?;
        let response = self
            .rest
            .request_mint_address(qid, dummy_kid)
            .await
            .map_err(Error::ClowderClient)?;
        Ok(response.address.assume_checked())
    }

    async fn request_to_pay_bill(
        &self,
        req: clowder_messages::RequestToPayEbillRequest,
        resp: clowder_messages::RequestToPayEbillResponse,
    ) -> Result<()> {
        self.nats
            .request_to_pay_bill(req, resp)
            .await
            .map(|_| ())
            .map_err(Error::ClowderClient)
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
        let serialized = borsh::to_vec(msg)?;
        let hash = bitcoin::hashes::sha256::Hash::hash(&serialized);
        let signature = self
            .signatory
            .sign_schnorr_hash(&hash.to_byte_array())
            .await?;
        let b64 = BASE64_STANDARD.encode(serialized);
        Ok((b64, signature))
    }

    async fn sign_onchain_melt_response(
        &self,
        msg: &wire_melt::MeltQuoteOnchainResponseBody,
    ) -> Result<(String, secp256k1::schnorr::Signature)> {
        let serialized = borsh::to_vec(msg)?;
        let hash = bitcoin::hashes::sha256::Hash::hash(&serialized);
        let signature = self
            .signatory
            .sign_schnorr_hash(&hash.to_byte_array())
            .await?;
        let b64 = BASE64_STANDARD.encode(serialized);
        Ok((b64, signature))
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
    ) -> Result<wire_melt::MeltTx> {
        let request = clowder_messages::MeltOnchainRequest {
            quote: qid,
            address: address.into_unchecked(),
            amount,
            inputs,
            commitment,
        };
        let response = self.nats.melt_onchain(request).await?;
        Ok(response.txid)
    }
}
