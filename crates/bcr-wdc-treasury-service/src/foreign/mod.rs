// ----- standard library imports
use std::collections::{HashMap, HashSet};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    wire::{keys as wire_keys, swap as wire_swap},
};
pub use bitcoin::{hashes::sha256::Hash as Sha256Hash, secp256k1};
use tracing::warn;
// ----- local modules
pub mod clients;
mod proof;
mod service;
pub mod settle;
// ----- local imports
use crate::{error::Result, TStamp};

// ----- end imports

pub use service::Service;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait OnlineRepository: Send + Sync {
    async fn store(&self, mint_id: secp256k1::PublicKey, proofs: Vec<cashu::Proof>) -> Result<()>;

    async fn store_htlc(
        &self,
        mint_id: secp256k1::PublicKey,
        hash: Sha256Hash,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()>;
    async fn search_htlc(
        &self,
        hash: &Sha256Hash,
    ) -> Result<Vec<(secp256k1::PublicKey, cashu::Proof)>>;
    async fn remove_htlcs(&self, ys: &[cashu::PublicKey]) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait OfflineRepository: Send + Sync {
    async fn store_fps(
        &self,
        mint_id: secp256k1::PublicKey,
        fps: Vec<wire_keys::ProofFingerprint>,
        hash: Vec<Sha256Hash>,
    ) -> Result<()>;
    async fn search_fp(
        &self,
        hash: &Sha256Hash,
    ) -> Result<Option<(secp256k1::PublicKey, wire_keys::ProofFingerprint)>>;
    async fn remove_fps(&self, ys: &[cashu::PublicKey]) -> Result<()>;
    async fn store_proofs(
        &self,
        mint_id: secp256k1::PublicKey,
        proof: Vec<cashu::Proof>,
    ) -> Result<()>;
    #[allow(dead_code)]
    async fn load_proofs(&self, mint_id: secp256k1::PublicKey) -> Result<Vec<cashu::Proof>>;
    #[allow(dead_code)]
    async fn remove_proofs(&self, ys: &[cashu::PublicKey]) -> Result<()>;
    async fn list_foreign_pks(&self) -> Result<Vec<secp256k1::PublicKey>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeysClient: Send + Sync {
    async fn get_keyset_with_expiration(
        &self,
        expiration: chrono::NaiveDate,
    ) -> Result<cashu::KeySet>;
    async fn sign(&self, blinds: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ClowderClient: Send + Sync {
    async fn get_mint_url_from_pk(&self, pk: &secp256k1::PublicKey) -> Result<reqwest::Url>;
    async fn get_myself_pk(&self) -> Result<secp256k1::PublicKey>;
    async fn sign_p2pk_proofs(&self, proofs: &[cashu::Proof]) -> Result<Vec<cashu::Proof>>;
    // yes if result is Ok
    async fn can_accept_offline_exchange(
        &self,
        fps: Vec<wire_keys::ProofFingerprint>,
    ) -> Result<(reqwest::Url, secp256k1::PublicKey)>;
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
    async fn check_htlc_proofs(
        &self,
        issuer: secp256k1::PublicKey,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()>;
    async fn signal_online_exchange_event(
        &self,
        inputs: Vec<cashu::Proof>,
        outputs: Vec<cashu::Proof>,
        path: Vec<secp256k1::PublicKey>,
    ) -> Result<Vec<cashu::Proof>>;
    async fn signal_offline_exchange_event(
        &self,
        inputs: Vec<wire_keys::ProofFingerprint>,
        hashes: Vec<Sha256Hash>,
        wallet_pk: cashu::PublicKey,
        outputs: Vec<cashu::Proof>,
    ) -> Result<()>;
    async fn sign_swap_commitment_request(
        &self,
        payload: wire_swap::SwapCommitmentRequest,
    ) -> Result<(String, secp256k1::schnorr::Signature)>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ForeignClient: Send + Sync {
    async fn prepare_swap_commitment_request(
        &self,
        inputs: Vec<cashu::Proof>,
        outputs: Vec<cashu::BlindedMessage>,
        now: TStamp,
    ) -> Result<wire_swap::SwapCommitmentRequest>;

    async fn commit_swap_with_signature(
        &self,
        payload: String,
        signature: secp256k1::schnorr::Signature,
    ) -> Result<(String, secp256k1::schnorr::Signature)>;

    async fn swap(
        &self,
        inputs: Vec<cashu::Proof>,
        outputs: Vec<cashu::BlindedMessage>,
        commitment: secp256k1::schnorr::Signature,
    ) -> Result<Vec<cashu::BlindSignature>>;

    async fn check_state(&self, ys: Vec<cashu::PublicKey>) -> Result<Vec<cashu::ProofState>>;
    async fn get_keyset(&self, kid: cashu::Id) -> Result<cashu::KeySet>;
    async fn list_keyset_infos(&self) -> Result<HashMap<cashu::Id, cashu::KeySetInfo>>;

    fn get_foreign_pk(&self) -> secp256k1::PublicKey;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait MintClientFactory: Send + Sync {
    async fn make_client(
        &self,
        mint_url: reqwest::Url,
        mint_pk: secp256k1::PublicKey,
    ) -> Result<Box<dyn ForeignClient>>;
}

fn to_mint_proofs_map(
    input: Vec<(secp256k1::PublicKey, cashu::Proof)>,
) -> HashMap<secp256k1::PublicKey, Vec<cashu::Proof>> {
    let mut map: HashMap<secp256k1::PublicKey, Vec<cashu::Proof>> = HashMap::new();
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
    for (fp, hash) in input.into_iter().zip(hashes) {
        map.entry(fp.keyset_id).or_default().push((fp, hash));
    }
    map
}

async fn signed_swap_with_foreign(
    foreign_proofs: Vec<cashu::Proof>,
    clowder: &dyn ClowderClient,
    foreign_cl: &dyn ForeignClient,
    now: TStamp,
) -> Result<Vec<cashu::Proof>> {
    let foreign_kids: HashSet<cashu::Id> = foreign_proofs.iter().map(|p| p.keyset_id).collect();
    let foreign_id = foreign_cl.get_foreign_pk();
    let mut foreign_kinfos = HashMap::new();
    for foreign_kid in foreign_kids {
        let kinfo = clowder.get_keyset_info(&foreign_id, &foreign_kid).await?;
        foreign_kinfos.insert(foreign_kid, kinfo);
    }
    // prepare the swap plan
    let swap_plan =
        bcr_common::core::swap::wallet::prepare_signed_swap(&foreign_proofs, &foreign_kinfos)?;
    let mut foreign_keysets_map: HashMap<cashu::Id, cashu::KeySet> = HashMap::new();
    let mut premint_secrets: Vec<cashu::PreMintSecrets> = Vec::with_capacity(swap_plan.len());
    for (kid, amount) in swap_plan {
        let keyset = clowder.get_keyset(&foreign_id, &kid).await?;
        let premint = cashu::PreMintSecrets::random(
            kid,
            amount,
            &cashu::amount::SplitTarget::None,
            &bcr_wdc_utils::keys::to_fee_and_amounts(&keyset),
        )?;
        premint_secrets.push(premint);
        foreign_keysets_map.insert(kid, keyset);
    }
    let blinds: Vec<cashu::BlindedMessage> = premint_secrets
        .iter()
        .flat_map(|p| p.blinded_messages())
        .collect();
    let premints: Vec<cashu::PreMint> = premint_secrets
        .into_iter()
        .flat_map(|p| p.secrets)
        .collect();
    // ready to swap using the signed commitment protocol
    let request = foreign_cl
        .prepare_swap_commitment_request(foreign_proofs.clone(), blinds.clone(), now)
        .await?;
    // sign the request with our clowder signatory service
    let (payload, signature) = clowder.sign_swap_commitment_request(request).await?;
    let (_foreign_payload, foreign_commitment) = foreign_cl
        .commit_swap_with_signature(payload, signature)
        .await?;
    let signatures = foreign_cl
        .swap(foreign_proofs, blinds, foreign_commitment)
        .await?;
    // unblind the signatures to get new proofs
    let mut new_proofs = Vec::with_capacity(signatures.len());
    for (signature, premint) in signatures.into_iter().zip(premints) {
        let keyset = foreign_keysets_map
            .get(&signature.keyset_id)
            .expect("keyset_id must be here");
        let amount = premint.amount;
        let Ok(new_p) =
            bcr_common::core::signature::unblind_ecash_signature(keyset, premint, signature)
        else {
            warn!("unblind_ecash_signature failed, lost {amount}");
            continue;
        };
        new_proofs.push(new_p);
    }
    Ok(new_proofs)
}
