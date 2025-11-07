// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::client::cdk::MintConnectorExt;
use bcr_wdc_utils::signatures::unblind_signatures;
use bitcoin::hashes::sha256::Hash as Sha256Hash;
use cashu::nut10 as cdk10;
// ----- local imports
use crate::{
    error::{Error, Result},
    TStamp,
};

// ----- end imports

pub fn extract_hash_timelock_from_htlc(p: &cashu::Proof) -> Result<(Sha256Hash, TStamp)> {
    let Ok(secret) = cdk10::Secret::try_from(p.secret.clone()) else {
        return Err(Error::InvalidInput(String::from("no spending condition")));
    };
    let Ok(conditions) = cashu::SpendingConditions::try_from(secret) else {
        return Err(Error::InvalidInput(String::from("no spending condition")));
    };
    let cashu::SpendingConditions::HTLCConditions { data, conditions } = conditions else {
        return Err(Error::InvalidInput(String::from("no HTLC conditions")));
    };
    let Some(cashu::Conditions { locktime, .. }) = conditions else {
        return Err(Error::InvalidInput(String::from("no HTLC side-conditions")));
    };
    let Some(locktime) = locktime else {
        return Err(Error::InvalidInput(String::from("no HTLC time tag")));
    };
    let locktime = TStamp::from_timestamp_secs(locktime as i64)
        .ok_or(Error::InvalidInput(String::from("invalid HTLC time tag")))?;
    Ok((data, locktime))
}

#[async_trait]
pub trait ClowderClient: Send + Sync {
    async fn check_htlc_proofs(
        &self,
        issuer: cashu::PublicKey,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()>;
}

/// check that all proofs:
/// - are unspent
/// - have same keyset_id, same htlc hash, same locktime
/// - perform check_htlc_foreign_proof
pub async fn check_htlc_foreign_proofs(
    issuer: cashu::PublicKey,
    proofs: &[cashu::Proof],
    mintcl: &dyn MintConnectorExt,
    clwdcl: &dyn ClowderClient,
) -> Result<(Sha256Hash, TStamp)> {
    if proofs.is_empty() {
        return Err(Error::InvalidInput(String::from("no proofs")));
    }
    clwdcl.check_htlc_proofs(issuer, proofs.to_vec()).await?;

    let fingerprints: Vec<cashu::PublicKey> = proofs
        .iter()
        .map(|p| p.y())
        .collect::<std::result::Result<_, _>>()?;
    let request = cashu::CheckStateRequest { ys: fingerprints };
    let response = mintcl.post_check_state(request).await?;
    let unspent = response
        .states
        .iter()
        .all(|s| matches!(s.state, cashu::nut07::State::Unspent));
    if !unspent {
        return Err(Error::InvalidInput(String::from(
            "One or more proofs are not unspent",
        )));
    }
    let (hash, locktime) = extract_hash_timelock_from_htlc(&proofs[0])?;
    Ok((hash, locktime))
}

#[async_trait]
pub trait KeysClient: Send + Sync {
    async fn sign(&self, blinds: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>>;
}

fn generate_htlc_secret(
    locktime: Option<TStamp>,
    hash: Sha256Hash,
    pk: cashu::PublicKey,
) -> Result<cdk10::Secret> {
    let conditions = cashu::Conditions::new(
        locktime.map(|t| t.timestamp() as u64),
        Some(vec![pk]),
        None,
        None,
        None,
        None,
    )?;
    let spending_conds = cashu::SpendingConditions::new_htlc(hash.to_string(), Some(conditions))?;
    let secret = cdk10::Secret::from(spending_conds);
    Ok(secret)
}

pub async fn generate_htlc_proofs(
    amount: cashu::Amount,
    locktime: Option<TStamp>,
    hash: Sha256Hash,
    pk: cashu::PublicKey,
    keyset: &cashu::KeySet,
    keycl: &dyn KeysClient,
) -> Result<Vec<cashu::Proof>> {
    let nut10secret = generate_htlc_secret(locktime, hash, pk)?;
    let secret = cashu::secret::Secret::try_from(nut10secret)?;
    let amounts = amount.split();
    let secrets = vec![secret; amounts.len()];
    let premints = cashu::PreMintSecrets::from_secrets(keyset.id, amounts, secrets)?;
    let blinds = premints.blinded_messages();
    let signatures = keycl.sign(&blinds).await?;
    let proofs = unblind_signatures(premints.iter(), signatures.into_iter(), &keyset)?;
    Ok(proofs)
}
