// ----- standard library imports
// ----- extra library imports
use bcr_common::wire::{exchange as wire_exchange, keys as wire_keys};
use bcr_wdc_webapi::{exchange as web_exchange, signatures as web_signatures, wallet as web_wallet};
use bitcoin::{hashes::sha256::Hash as Sha256Hash, secp256k1::schnorr::Signature};
use thiserror::Error;
use uuid::Uuid;
// ----- local modules
// ----- local imports
pub use reqwest::Url;

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
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

    pub const GENERATEBLINDS_EP_V1: &'static str = "/v1/admin/treasury/credit/generate_blinds";
    #[cfg(feature = "authorized")]
    pub async fn generate_blinds(
        &self,
        kid: cashu::Id,
        amount: cashu::Amount,
    ) -> Result<(Uuid, Vec<cashu::BlindedMessage>)> {
        let msg = web_signatures::GenerateBlindedMessagesRequest { kid, total: amount };
        let url = self
            .base
            .join(Self::GENERATEBLINDS_EP_V1)
            .expect("generate_blinds relative path");
        let request = self.cl.post(url).json(&msg);
        let response: web_signatures::GenerateBlindedMessagesResponse =
            self.auth.authorize(request).send().await?.json().await?;
        Ok((response.request_id, response.messages))
    }

    pub const STORESIGNATURES_EP_V1: &'static str = "/v1/admin/treasury/credit/store_signatures";
    #[cfg(feature = "authorized")]
    pub async fn store_signatures(
        &self,
        rid: uuid::Uuid,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<()> {
        let msg = web_signatures::StoreBlindSignaturesRequest { rid, signatures };
        let url = self
            .base
            .join(Self::STORESIGNATURES_EP_V1)
            .expect("store_signatures relative path");
        let request = self.cl.post(url).json(&msg);
        let response = self.auth.authorize(request).send().await?;
        response.error_for_status()?;
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

    pub const CRSATBALANCE_EP_V1: &'static str = "/v1/admin/treasury/credit/balance";
    #[cfg(feature = "authorized")]
    pub async fn crsat_balance(&self) -> Result<web_wallet::ECashBalance> {
        let url = self
            .base
            .join(Self::CRSATBALANCE_EP_V1)
            .expect("crsat balance relative path");
        let request = self.cl.get(url);
        let response: web_wallet::ECashBalance =
            self.auth.authorize(request).send().await?.json().await?;
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
    pub async fn sat_exchange_online(
        &self,
        proofs: Vec<cashu::Proof>,
        exchange_path: Vec<secp256k1::PublicKey>,
    ) -> Result<Vec<cashu::Proof>> {
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
        Ok(response.proofs)
    }

    pub const CRSATEXCHANGEONLINE_EP_V1: &'static str = "/v1/treasury/credit/exchange/online";
    pub async fn crsat_exchange_online(
        &self,
        proofs: Vec<cashu::Proof>,
        exchange_path: Vec<secp256k1::PublicKey>,
    ) -> Result<Vec<cashu::Proof>> {
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
        Ok(response.proofs)
    }

    pub const SATEXCHANGEOFFLINE_EP_V1: &'static str = "/v1/treasury/debit/exchange/offline";
    pub async fn sat_exchange_offline(
        &self,
        fingerprints: Vec<wire_keys::ProofFingerprint>,
        hashes: Vec<Sha256Hash>,
        wallet_pk: cashu::PublicKey,
        mint_pk: secp256k1::PublicKey,
    ) -> Result<(Vec<cashu::Proof>, Signature)> {
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
    pub async fn crsat_exchange_offline(
        &self,
        fingerprints: Vec<wire_keys::ProofFingerprint>,
        hashes: Vec<Sha256Hash>,
        wallet_pk: cashu::PublicKey,
        mint_pk: secp256k1::PublicKey,
    ) -> Result<(Vec<cashu::Proof>, Signature)> {
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

}
