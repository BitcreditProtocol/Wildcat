// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    client::clowder::ClowderNatsClient,
    client::{admin::clowder::Client as ClowderRestClient, core::Client as CoreClient},
    core::{maturity::active_keyset, signature, CURRENCY_UNIT},
    ecash,
    wire::{
        attestation::AttestedFingerprints, clowder as wire_clowder, keys as wire_keys,
        melt as wire_melt, mint as wire_mint,
    },
};
use bitcoin::secp256k1::PublicKey;
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    onchain::{ClowderClient, MeltOnchainOrder, VaultService, WildcatClient},
    vault, TStamp,
};

// ----- end imports

#[derive(Clone, Debug)]
pub struct WildcatCl {
    /// core-service's public (web) endpoints, e.g. `keys`, `check_state`.
    pub core_cl: Arc<CoreClient>,
    /// core-service's admin-only endpoints, e.g. `sign`, `burn`, `reserve`.
    pub core_admin_cl: Arc<CoreClient>,
}

#[async_trait]
impl WildcatClient for WildcatCl {
    async fn sign(&self, blinds: Vec<cashu::BlindedMessage>) -> Result<Vec<cashu::BlindSignature>> {
        let c_blinds: Vec<_> = blinds.into_iter().map(From::from).collect();
        let signatures = self.core_admin_cl.sign(&c_blinds).await?;
        Ok(signatures.into_iter().map(From::from).collect())
    }

    async fn burn(&self, inputs: Vec<cashu::Proof>) -> Result<()> {
        let c_inputs = inputs.into_iter().map(From::from).collect();
        self.core_admin_cl.burn(c_inputs).await?;
        Ok(())
    }

    async fn recover(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        let c_proofs = proofs.into_iter().map(From::from).collect();
        self.core_admin_cl.recover(c_proofs).await?;
        Ok(())
    }

    async fn reserve_inputs(&self, inputs: Vec<cashu::PublicKey>, deadline: TStamp) -> Result<()> {
        let c_inputs = inputs
            .iter()
            .map(|y| {
                PublicKey::from_slice(&y.to_bytes()).expect("cashu::PublicKey <-> secp::PublicKey")
            })
            .collect();
        self.core_admin_cl.reserve(c_inputs, deadline).await?;
        Ok(())
    }

    async fn keyset_info(&self, kid: cashu::Id) -> Result<ecash::KeySetInfo> {
        let info = self.core_cl.keyset_info(kid).await?;
        Ok(info)
    }

    async fn keyset(&self, kid: cashu::Id) -> Result<ecash::KeySet> {
        let keyset = self.core_cl.keys(kid).await?;
        Ok(keyset)
    }

    async fn get_active_keyset(&self) -> Result<cashu::Id> {
        let filter = wire_keys::KeysetInfoFilters {
            unit: Some(CURRENCY_UNIT),
            ..Default::default()
        };
        let infos = self.core_cl.list_keyset_info(filter).await?;
        let now = TStamp::now_utc().unix_timestamp() as u64;
        active_keyset(&infos, true, now)
            .map(|info| info.id.into())
            .ok_or(Error::Internal(String::from(
                "no active debit keyset found",
            )))
    }

    async fn verify_fingerprints(&self, fps: &[wire_keys::ProofFingerprint]) -> Result<()> {
        for fp in fps {
            self.core_admin_cl.verify_fingerprint(fp).await?;
        }
        Ok(())
    }

    async fn verify_proofs(&self, ps: &[cashu::Proof]) -> Result<()> {
        for p in ps {
            let c_proof = ecash::Proof::from(p.clone());
            self.core_admin_cl.verify_proof(&c_proof).await?;
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
        msg: &wire_mint::OnchainMintQuoteResponseBodyV1,
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
        let (content, _) = signature::serialize_borsh_msg_b64(msg)?;
        Ok((content, response.commitment))
    }

    async fn sign_onchain_melt_response(
        &self,
        msg: &wire_melt::MeltQuoteOnchainResponseBody,
    ) -> Result<(String, secp256k1::schnorr::Signature)> {
        let request = wire_clowder::MeltQuoteOnchainRequest {
            quote_id: msg.quote,
            inputs: msg.inputs.clone(),
            address: msg.address.clone(),
            admin_fees: cashu::Amount::from(msg.melt_fee.to_sat()),
            network_fees: msg.network_fee,
            expiry: msg.expiry,
            wallet_key: msg.wallet_key,
        };
        let response = self.nats.melt_quote_onchain(request).await?;
        let (content, _) = signature::serialize_borsh_msg_b64(msg)?;
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

    async fn melt_onchain(&self, req: MeltOnchainOrder) -> Result<bitcoin::Txid> {
        let request = wire_clowder::MeltOnchainRequest {
            quote: req.qid,
            address: req.address.into_unchecked(),
            amount: req.target,
            inputs: req.inputs,
            commitment: req.commitment,
            fees: req.fees,
            network_fee: Some(req.network_fee),
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

    async fn estimate_onchain_tx(
        &self,
        amount: bitcoin::Amount,
        address: Option<bitcoin::Address<bitcoin::address::NetworkUnchecked>>,
    ) -> Result<wire_clowder::OnchainTxEstimateResponse> {
        let response = self.rest.onchain_tx_estimate(amount, address).await?;
        Ok(response)
    }

    async fn get_onchain_reserve(&self) -> Result<bitcoin::Amount> {
        let collaterals = self.rest.get_mint_collateral().await?;
        Ok(collaterals.onchain)
    }

    async fn authenticate_attestation(
        &self,
        alpha_id: &PublicKey,
        inputs: &AttestedFingerprints,
    ) -> Result<()> {
        bcr_wdc_utils::attestation::authenticate_with_betas(&self.rest, alpha_id, inputs).await?;
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

#[cfg(feature = "test-utils")]
pub struct DummyWildcatClient;

#[cfg(feature = "test-utils")]
#[async_trait]
impl WildcatClient for DummyWildcatClient {
    async fn verify_fingerprints(&self, _fps: &[wire_keys::ProofFingerprint]) -> Result<()> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn verify_proofs(&self, _proofs: &[cashu::Proof]) -> Result<()> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn check_spendable(
        &self,
        _proofs: Vec<cashu::PublicKey>,
    ) -> Result<Vec<cashu::ProofState>> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn sign(&self, _blinds: Vec<cashu::BlindedMessage>) -> Result<Vec<cashu::BlindSignature>> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn burn(&self, _inputs: Vec<cashu::Proof>) -> Result<()> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn recover(&self, _inputs: Vec<cashu::Proof>) -> Result<()> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn reserve_inputs(
        &self,
        _inputs: Vec<cashu::PublicKey>,
        _deadline: TStamp,
    ) -> Result<()> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn keyset_info(&self, _kid: cashu::Id) -> Result<ecash::KeySetInfo> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn keyset(&self, _kid: cashu::Id) -> Result<ecash::KeySet> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn get_active_keyset(&self) -> Result<cashu::Id> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
}

#[cfg(feature = "test-utils")]
pub struct DummyClowderClient;

#[cfg(feature = "test-utils")]
#[async_trait]
impl ClowderClient for DummyClowderClient {
    async fn request_to_pay_bill(
        &self,
        _req: wire_clowder::RequestToPayEbillRequest,
        _resp: wire_clowder::RequestToPayEbillResponse,
    ) -> Result<()> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn request_onchain_mint_address(
        &self,
        _qid: Uuid,
        _kid: cashu::Id,
    ) -> Result<bitcoin::Address> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn verify_onchain_mint_payment(
        &self,
        _qid: Uuid,
        _kid: cashu::Id,
    ) -> Result<bitcoin::Amount> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn mint_onchain(
        &self,
        _qid: Uuid,
        _kid: cashu::Id,
        _signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn sign_onchain_mint_response(
        &self,
        _msg: &wire_mint::OnchainMintQuoteResponseBodyV1,
    ) -> Result<(String, secp256k1::schnorr::Signature)> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn sign_onchain_melt_response(
        &self,
        _msg: &wire_melt::MeltQuoteOnchainResponseBody,
    ) -> Result<(String, secp256k1::schnorr::Signature)> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn verify_onchain_address(
        &self,
        _address: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    ) -> Result<bitcoin::Address> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn melt_onchain(&self, _req: MeltOnchainOrder) -> Result<bitcoin::Txid> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn fetch_mint_signatures(
        &self,
        _qid: Uuid,
        _mint_id: secp256k1::PublicKey,
    ) -> Result<Vec<cashu::BlindSignature>> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn estimate_onchain_tx(
        &self,
        _amount: bitcoin::Amount,
        _address: Option<bitcoin::Address<bitcoin::address::NetworkUnchecked>>,
    ) -> Result<wire_clowder::OnchainTxEstimateResponse> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn get_onchain_reserve(&self) -> Result<bitcoin::Amount> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
    async fn authenticate_attestation(
        &self,
        _alpha_id: &PublicKey,
        _inputs: &AttestedFingerprints,
    ) -> Result<()> {
        Err(Error::Internal(String::from("test_utils dummy: not implemented")))
    }
}
