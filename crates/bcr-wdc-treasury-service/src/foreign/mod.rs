// ----- standard library imports
use std::collections::HashMap;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, cdk, wire::keys as wire_keys, wire::swap as wire_swap};
pub use bitcoin::hashes::sha256::Hash as Sha256Hash;
// ----- local modules
pub mod clients;
pub mod crsat;
mod proof;
pub mod sat;
pub mod settle;
// ----- local imports
use crate::error::Result;

// ----- end imports

#[async_trait]
pub trait MintConnectorExt: cdk::wallet::MintConnector + Send + Sync {
    async fn swap(
        &self,
        request: wire_swap::SwapRequest,
    ) -> std::result::Result<wire_swap::SwapResponse, cdk::Error>;
}

#[cfg(test)]
pub mod test_utils {
    use async_trait::async_trait;
    use bcr_common::{cashu, cdk};

    type CdkResult<T> = std::result::Result<T, cdk::Error>;

    mockall::mock! {
        pub MintConnector {
        }
        impl std::fmt::Debug for MintConnector {
            fn fmt<'a>(&self, f: &mut std::fmt::Formatter<'a>) -> std::fmt::Result;
        }

        #[async_trait]
        impl cdk::wallet::MintConnector for MintConnector {
            async fn get_mint_keys(&self) -> CdkResult<Vec<cashu::KeySet>>;
            async fn get_mint_keyset(&self, keyset_id: cashu::Id) -> CdkResult<cashu::KeySet>;
            async fn get_mint_keysets(&self) -> CdkResult<cashu::KeysetResponse>;
            async fn post_mint_quote(
                &self,
                request: cashu::MintQuoteBolt11Request,
            ) -> CdkResult<cashu::MintQuoteBolt11Response<String>>;
            async fn get_mint_quote_status(
                &self,
                quote_id: &str,
            ) -> CdkResult<cashu::MintQuoteBolt11Response<String>>;
            async fn post_mint(&self, request: cashu::MintRequest<String>) -> CdkResult<cashu::MintResponse>;
            async fn post_melt_quote(
                &self,
                request: cashu::MeltQuoteBolt11Request,
            ) -> CdkResult<cashu::MeltQuoteBolt11Response<String>>;
            async fn get_melt_quote_status(
                &self,
                quote_id: &str,
            ) -> CdkResult<cashu::MeltQuoteBolt11Response<String>>;
            async fn post_melt(
                &self,
                request: cashu::MeltRequest<String>,
            ) -> CdkResult<cashu::MeltQuoteBolt11Response<String>>;
            async fn post_swap(&self, request: cashu::SwapRequest) -> CdkResult<cashu::SwapResponse>;
            async fn get_mint_info(&self) -> CdkResult<cashu::MintInfo>;
            async fn post_check_state(
                &self,
                request: cashu::CheckStateRequest,
            ) -> CdkResult<cashu::CheckStateResponse>;
            async fn post_restore(&self, request: cashu::RestoreRequest) -> CdkResult<cashu::RestoreResponse>;
            async fn post_mint_bolt12_quote(
                &self,
                request: cashu::MintQuoteBolt12Request,
            ) -> CdkResult<cashu::MintQuoteBolt12Response<String>>;
            async fn get_mint_quote_bolt12_status(
                &self,
                quote_id: &str,
            ) -> CdkResult<cashu::MintQuoteBolt12Response<String>>;
            async fn post_melt_bolt12_quote(
                &self,
                request: cashu::MeltQuoteBolt12Request,
            ) -> CdkResult<cashu::MeltQuoteBolt11Response<String>>;
            async fn get_melt_bolt12_quote_status(
                &self,
                quote_id: &str,
            ) -> CdkResult<cashu::MeltQuoteBolt11Response<String>>;
            async fn post_melt_bolt12(
                &self,
                request: cashu::MeltRequest<String>,
            ) -> CdkResult<cashu::MeltQuoteBolt11Response<String>>;
        }

        #[async_trait]
        impl super::MintConnectorExt for MintConnector {
            async fn swap(
                &self,
                request: bcr_common::wire::swap::SwapRequest,
            ) -> std::result::Result<bcr_common::wire::swap::SwapResponse, cdk::Error>;
        }
    }
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait OnlineRepository: Send + Sync {
    async fn store(
        &self,
        mint: (secp256k1::PublicKey, cashu::MintUrl),
        proofs: Vec<cashu::Proof>,
    ) -> Result<()>;
    #[allow(dead_code)]
    async fn list(&self) -> Result<Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>>;

    async fn store_htlc(
        &self,
        mint: (secp256k1::PublicKey, cashu::MintUrl),
        hash: Sha256Hash,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()>;
    async fn search_htlc(
        &self,
        hash: &Sha256Hash,
    ) -> Result<Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>>;
    async fn remove_htlcs(&self, ys: &[cashu::PublicKey]) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait OfflineRepository: Send + Sync {
    async fn store_fps(
        &self,
        alpha: (secp256k1::PublicKey, cashu::MintUrl),
        fps: Vec<wire_keys::ProofFingerprint>,
        hash: Vec<Sha256Hash>,
    ) -> Result<()>;
    async fn search_fp(
        &self,
        hash: &Sha256Hash,
    ) -> Result<
        Option<(
            (secp256k1::PublicKey, cashu::MintUrl),
            wire_keys::ProofFingerprint,
        )>,
    >;
    async fn remove_fps(&self, ys: &[cashu::PublicKey]) -> Result<()>;
    async fn store_proofs(
        &self,
        alpha: (secp256k1::PublicKey, cashu::MintUrl),
        proof: Vec<cashu::Proof>,
    ) -> Result<()>;
    #[allow(dead_code)]
    async fn load_proofs(
        &self,
        alpha: &(secp256k1::PublicKey, cashu::MintUrl),
    ) -> Result<Vec<cashu::Proof>>;
    #[allow(dead_code)]
    async fn remove_proofs(&self, ys: &[cashu::PublicKey]) -> Result<()>;
}

#[async_trait]
pub trait ClowderClient: proof::ClowderClient {
    async fn get_mint_url_from_pk(&self, pk: &cashu::PublicKey) -> Result<cashu::MintUrl>;
    async fn get_myself_pk(&self) -> Result<bitcoin::PublicKey>;
    async fn sign_p2pk_proofs(&self, proofs: &[cashu::Proof]) -> Result<Vec<cashu::Proof>>;
    // yes if result is Ok
    async fn can_accept_offline_exchange(
        &self,
        fps: Vec<wire_keys::ProofFingerprint>,
    ) -> Result<(cashu::MintUrl, secp256k1::PublicKey)>;
    async fn get_keyset_info(
        &self,
        alpha_pk: &secp256k1::PublicKey,
        kid: &cashu::Id,
    ) -> Result<cashu::KeySetInfo>;
    async fn get_keyset(
        &self,
        alpha_pk: &secp256k1::PublicKey,
        kid: &cashu::Id,
    ) -> Result<cashu::KeySet>;
    async fn is_offline(&self, pk: secp256k1::PublicKey) -> Result<bool>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MintClientFactory: Send + Sync {
    async fn make_client(&self, mint_url: cashu::MintUrl) -> Result<Box<dyn MintConnectorExt>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait OfflineSettleHandler: Send + Sync {
    fn monitor(&self, mint: (secp256k1::PublicKey, cashu::MintUrl)) -> Result<()>;
    async fn stop(&self) -> Result<()>;
}

fn proofs_vec_to_map(
    input: Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>,
) -> HashMap<(secp256k1::PublicKey, cashu::MintUrl), Vec<cashu::Proof>> {
    let mut map: HashMap<(secp256k1::PublicKey, cashu::MintUrl), Vec<cashu::Proof>> =
        HashMap::new();
    for (mint, proof) in input {
        map.entry(mint).or_default().push(proof);
    }
    map
}

fn fingerprints_vec_to_map(
    input: Vec<wire_keys::ProofFingerprint>,
    hashes: Vec<Sha256Hash>,
) -> HashMap<cashu::Id, Vec<(wire_keys::ProofFingerprint, Sha256Hash)>> {
    let mut map: HashMap<cashu::Id, Vec<(wire_keys::ProofFingerprint, Sha256Hash)>> =
        HashMap::new();
    for (fp, hash) in input.into_iter().zip(hashes.into_iter()) {
        map.entry(fp.keyset_id).or_default().push((fp, hash));
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result;
    use async_trait::async_trait;
    use bcr_common::wire::keys as wire_keys;

    mockall::mock! {
        pub ClowderClient{
        }

        #[async_trait]
        impl super::proof::ClowderClient for ClowderClient {
            async fn check_htlc_proofs(
                &self,
                issuer: cashu::PublicKey,
                proofs: Vec<cashu::Proof>,
            ) -> Result<()>;
        }
        #[async_trait]
        impl super::ClowderClient for ClowderClient {
            async fn get_mint_url_from_pk(&self, pk: &cashu::PublicKey) -> Result<cashu::MintUrl>;
            async fn get_myself_pk(&self) -> Result<bitcoin::PublicKey>;
            async fn sign_p2pk_proofs(&self, proofs: &[cashu::Proof]) -> Result<Vec<cashu::Proof>>;
            async fn can_accept_offline_exchange(
                &self,
                fps: Vec<wire_keys::ProofFingerprint>,
            ) -> Result<(cashu::MintUrl, secp256k1::PublicKey)>;
            async fn get_keyset_info(
                &self,
                alpha_pk: &secp256k1::PublicKey,
                kid: &cashu::Id,
            ) -> Result<cashu::KeySetInfo>;
            async fn get_keyset(
                &self,
                alpha_pk: &secp256k1::PublicKey,
                kid: &cashu::Id,
            ) -> Result<cashu::KeySet>;
            async fn is_offline(&self, pk: secp256k1::PublicKey) -> Result<bool>;
        }
    }
}
