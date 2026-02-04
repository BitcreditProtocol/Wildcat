// ----- standard library imports
// ----- extra library imports
use bcr_common::{
    cashu,
    core::BillId,
    wire::{exchange as wire_exchange, keys as wire_keys, signatures as wire_signatures},
};
use bcr_wdc_webapi::{exchange as web_exchange, wallet as web_wallet};
use bitcoin::{hashes::sha256::Hash as Sha256Hash, secp256k1::schnorr::Signature, Amount};
use thiserror::Error;
// ----- local modules
// ----- local imports
pub use reqwest::Url;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("resource not found {0}")]
    ResourceNotFound(String),
    #[error("internal error {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("signature verification {0}")]
    Signature(#[from] bcr_common::core::signature::BorshMsgSignatureError),
}

#[derive(Debug, Clone)]
pub struct TreasuryClient {
    cl: reqwest::Client,
    base: reqwest::Url,
    #[cfg(feature = "authorized")]
    auth: bcr_wdc_utils::client::AuthorizationPlugin,
}

impl TreasuryClient {
    pub fn new(base: reqwest::Url) -> Self {
        Self {
            cl: reqwest::Client::new(),
            base,
            #[cfg(feature = "authorized")]
            auth: Default::default(),
        }
    }

    #[cfg(feature = "authorized")]
    pub async fn authenticate(
        &mut self,
        token_url: Url,
        client_id: &str,
        client_secret: &str,
        username: &str,
        password: &str,
    ) -> Result<()> {
        self.auth
            .authenticate(
                self.cl.clone(),
                token_url,
                client_id,
                client_secret,
                username,
                password,
            )
            .await?;
        Ok(())
    }

    pub const REDEEM_EP_V1: &'static str = "/v1/treasury/redeem";
    pub async fn redeem(
        &self,
        inputs: Vec<cashu::Proof>,
        outputs: Vec<cashu::BlindedMessage>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        let msg = cashu::SwapRequest::new(inputs, outputs);
        let url = self
            .base
            .join(Self::REDEEM_EP_V1)
            .expect("redeem relative path");
        let request = self.cl.post(url).json(&msg);
        let response: cashu::SwapResponse = request.send().await?.json().await?;
        Ok(response.signatures)
    }

    pub const REQTOPAY_EP_V1: &str = "/v1/admin/treasury/debit/request_to_pay_ebill";
    #[cfg(feature = "authorized")]
    pub async fn request_to_pay_ebill(
        &self,
        ebill_id: BillId,
        amount: Amount,
        deadline: chrono::DateTime<chrono::Utc>,
    ) -> Result<wire_signatures::RequestToMintFromEBillResponse> {
        let request = wire_signatures::RequestToMintFromEBillRequest {
            ebill_id,
            amount,
            deadline,
        };
        let url = self
            .base
            .join(Self::REQTOPAY_EP_V1)
            .expect("request_to_pay_ebill relative path");
        let req = self.cl.post(url).json(&request);
        let response: wire_signatures::RequestToMintFromEBillResponse =
            self.auth.authorize(req).send().await?.json().await?;
        Ok(response)
    }

    pub const SATBALANCE_EP_V1: &'static str = "/v1/admin/treasury/debit/balance";
    #[cfg(feature = "authorized")]
    pub async fn sat_balance(&self) -> Result<web_wallet::ECashBalance> {
        let url = self
            .base
            .join(Self::SATBALANCE_EP_V1)
            .expect("sat balance relative path");
        let request = self.cl.get(url);
        let response: web_wallet::ECashBalance =
            self.auth.authorize(request).send().await?.json().await?;
        Ok(response)
    }

    pub const SATEXCHANGEONLINE_EP_V1: &'static str = "/v1/treasury/debit/exchange/online";
    pub async fn sat_exchange_online_raw(
        &self,
        proofs: Vec<cashu::Proof>,
        exchange_path: Vec<secp256k1::PublicKey>,
    ) -> Result<wire_exchange::OnlineExchangeResponse> {
        let url = self
            .base
            .join(Self::SATEXCHANGEONLINE_EP_V1)
            .expect("sat_exchange_online relative path");
        let msg = wire_exchange::OnlineExchangeRequest {
            proofs,
            exchange_path,
        };
        let request = self.cl.post(url).json(&msg);
        let response: wire_exchange::OnlineExchangeResponse = request.send().await?.json().await?;
        Ok(response)
    }

    pub async fn sat_exchange_online(
        &self,
        proofs: Vec<cashu::Proof>,
        exchange_path: Vec<secp256k1::PublicKey>,
    ) -> Result<Vec<cashu::Proof>> {
        let response = self.sat_exchange_online_raw(proofs, exchange_path).await?;
        Ok(response.proofs)
    }

    pub const CRSATEXCHANGEONLINE_EP_V1: &'static str = "/v1/treasury/credit/exchange/online";
    pub async fn crsat_exchange_online_raw(
        &self,
        proofs: Vec<cashu::Proof>,
        exchange_path: Vec<secp256k1::PublicKey>,
    ) -> Result<wire_exchange::OnlineExchangeResponse> {
        let url = self
            .base
            .join(Self::CRSATEXCHANGEONLINE_EP_V1)
            .expect("crsat_exchange_online relative path");
        let msg = wire_exchange::OnlineExchangeRequest {
            proofs,
            exchange_path,
        };
        let request = self.cl.post(url).json(&msg);
        let response: wire_exchange::OnlineExchangeResponse = request.send().await?.json().await?;
        Ok(response)
    }

    pub async fn crsat_exchange_online(
        &self,
        proofs: Vec<cashu::Proof>,
        exchange_path: Vec<secp256k1::PublicKey>,
    ) -> Result<Vec<cashu::Proof>> {
        let response = self
            .crsat_exchange_online_raw(proofs, exchange_path)
            .await?;
        Ok(response.proofs)
    }

    pub const SATEXCHANGEOFFLINE_EP_V1: &'static str = "/v1/treasury/debit/exchange/offline";
    pub async fn sat_exchange_offline_raw(
        &self,
        fingerprints: Vec<wire_keys::ProofFingerprint>,
        hashes: Vec<Sha256Hash>,
        wallet_pk: cashu::PublicKey,
    ) -> Result<wire_exchange::OfflineExchangeResponse> {
        let url = self
            .base
            .join(Self::SATEXCHANGEOFFLINE_EP_V1)
            .expect("sat_exchange_offline relative path");
        let msg = wire_exchange::OfflineExchangeRequest {
            fingerprints,
            hashes,
            wallet_pk,
        };
        let request = self.cl.post(url).json(&msg);
        let response: wire_exchange::OfflineExchangeResponse = request.send().await?.json().await?;
        Ok(response)
    }

    pub async fn sat_exchange_offline(
        &self,
        fingerprints: Vec<wire_keys::ProofFingerprint>,
        hashes: Vec<Sha256Hash>,
        wallet_pk: cashu::PublicKey,
        mint_pk: secp256k1::PublicKey,
    ) -> Result<(Vec<cashu::Proof>, Signature)> {
        let response = self
            .sat_exchange_offline_raw(fingerprints, hashes, wallet_pk)
            .await?;
        bcr_common::core::signature::schnorr_verify_b64(
            &response.content,
            &response.signature,
            &mint_pk.x_only_public_key().0,
        )?;
        let payload: wire_exchange::OfflineExchangePayload =
            bcr_common::core::signature::deserialize_borsh_msg(&response.content)?;
        Ok((payload.proofs, response.signature))
    }

    pub const CRSATEXCHANGEOFFLINE_EP_V1: &'static str = "/v1/treasury/credit/exchange/offline";
    pub async fn crsat_exchange_offline_raw(
        &self,
        fingerprints: Vec<wire_keys::ProofFingerprint>,
        hashes: Vec<Sha256Hash>,
        wallet_pk: cashu::PublicKey,
    ) -> Result<wire_exchange::OfflineExchangeResponse> {
        let url = self
            .base
            .join(Self::CRSATEXCHANGEOFFLINE_EP_V1)
            .expect("crsat_exchange_offline relative path");
        let msg = wire_exchange::OfflineExchangeRequest {
            fingerprints,
            hashes,
            wallet_pk,
        };
        let request = self.cl.post(url).json(&msg);
        let response: wire_exchange::OfflineExchangeResponse = request.send().await?.json().await?;
        Ok(response)
    }

    pub async fn crsat_exchange_offline(
        &self,
        fingerprints: Vec<wire_keys::ProofFingerprint>,
        hashes: Vec<Sha256Hash>,
        wallet_pk: cashu::PublicKey,
        mint_pk: secp256k1::PublicKey,
    ) -> Result<(Vec<cashu::Proof>, Signature)> {
        let response = self
            .crsat_exchange_offline_raw(fingerprints, hashes, wallet_pk)
            .await?;
        bcr_common::core::signature::schnorr_verify_b64(
            &response.content,
            &response.signature,
            &mint_pk.x_only_public_key().0,
        )?;
        let payload: wire_exchange::OfflineExchangePayload =
            bcr_common::core::signature::deserialize_borsh_msg(&response.content)?;
        Ok((payload.proofs, response.signature))
    }

    pub const TRYSATHTLC_EP_V1: &'static str = "/v1/admin/treasury/debit/try_htlc_swap";
    #[cfg(feature = "authorized")]
    pub async fn try_sat_htlc(&self, preimage: String) -> Result<cashu::Amount> {
        let url = self
            .base
            .join(Self::TRYSATHTLC_EP_V1)
            .expect("try_sat_htlc relative path");
        let msg = web_exchange::HtlcSwapAttemptRequest { preimage };
        let request = self.cl.post(url).json(&msg);
        let response = self.auth.authorize(request).send().await?.json().await?;
        Ok(response)
    }

    pub const TRYCRSATHTLC_EP_V1: &'static str = "/v1/admin/treasury/credit/try_htlc_swap";
    #[cfg(feature = "authorized")]
    pub async fn try_crsat_htlc(&self, preimage: String) -> Result<cashu::Amount> {
        let url = self
            .base
            .join(Self::TRYCRSATHTLC_EP_V1)
            .expect("try_crsat_htlc relative path");
        let msg = web_exchange::HtlcSwapAttemptRequest { preimage };
        let request = self.cl.post(url).json(&msg);
        let response = self.auth.authorize(request).send().await?.json().await?;
        Ok(response)
    }

    pub const IS_EBILL_MINT_COMPLETE_EP_V1: &'static str =
        "/v1/admin/treasury/debit/mint_complete/{ebill_id}";
    pub async fn is_ebill_mint_complete(&self, ebill_id: BillId) -> Result<bool> {
        let path = Self::IS_EBILL_MINT_COMPLETE_EP_V1.replace("{ebill_id}", &ebill_id.to_string());
        let url = self
            .base
            .join(&path)
            .expect("is_ebill_mint_complete relative path");
        let request = self.cl.get(url);
        let response = request.send().await?;
        if matches!(response.status(), reqwest::StatusCode::NOT_FOUND) {
            return Err(Error::ResourceNotFound(ebill_id.to_string()));
        }
        let response: web_wallet::EbillPaymentComplete = response.json().await?;
        Ok(response.complete)
    }

    pub const MELTQUOTE_ONCHAIN_EP_V1: &'static str = "/v1/melt/quote/onchain";
    pub const MELT_ONCHAIN_EP_V1: &'static str = "/v1/melt/onchain";
    pub const MINTQUOTE_ONCHAIN_EP_V1: &'static str = "/v1/mint/quote/onchain";
    pub const MINTQUOTE_ONCHAIN_GET_EP_V1: &'static str = "/v1/mint/quote/onchain/{quote_id}";
    pub const MINT_ONCHAIN_EP_V1: &'static str = "/v1/mint/onchain";
}
