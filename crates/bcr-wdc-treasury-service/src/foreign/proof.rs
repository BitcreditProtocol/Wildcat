// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_utils::signatures::unblind_signatures;
use cashu::nut10 as cdk10;
// ----- local imports
use crate::{
    error::{Error, Result},
    TStamp,
};

// ----- end imports

pub fn extract_hash_timelock_from_htlc(p: &cashu::Proof) -> Result<(String, TStamp)> {
    let Ok(secret) = cdk10::Secret::try_from(p.secret.clone()) else {
        return Err(Error::InvalidInput(String::from("no spending condition")));
    };
    if !matches!(secret.kind(), cdk10::Kind::HTLC) {
        return Err(Error::InvalidInput(String::from("non-HTLC")));
    }
    let hash = secret.secret_data().data().to_owned();
    for tag_group in secret.secret_data().tags().unwrap_or(&vec![]) {
        let empty = String::new();
        let tag = tag_group.first().unwrap_or(&empty);
        if tag == "locktime" && tag_group.len() > 2 {
            let locktime = tag_group[1]
                .parse::<i64>()
                .map_err(|_| Error::InvalidInput(String::from("invalid HTLC time tag")))?;
            let locktime = TStamp::from_timestamp_secs(locktime)
                .ok_or(Error::InvalidInput(String::from("invalid HTLC time tag")))?;
            return Ok((hash, locktime));
        }
    }
    Err(Error::InvalidInput(String::from("no HTLC time tag")))
}

#[async_trait]
pub trait ClowderClient: Send + Sync {
    async fn check_htlc_proofs(
        &self,
        issuer: cashu::PublicKey,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()>;
}

pub trait MintConnectorExt: cdk::wallet::MintConnector + Send + Sync {}
impl MintConnectorExt for cdk::wallet::HttpClient {}
/// check that all proofs:
/// - are unspent
/// - have same keyset_id, same htlc hash, same locktime
/// - perform check_htlc_foreign_proof
pub async fn check_htlc_foreign_proofs(
    issuer: cashu::PublicKey,
    proofs: &[cashu::Proof],
    mintcl: &dyn MintConnectorExt,
    clwdcl: &dyn ClowderClient,
) -> Result<(String, TStamp)> {
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

fn generate_htlc_secret(locktime: TStamp, hash: &str, pk: cashu::PublicKey) -> cdk10::Secret {
    let tags = vec![
        vec![String::from("pubkeys"), pk.to_string()],
        vec![String::from("locktime"), locktime.timestamp().to_string()],
    ];
    cdk10::Secret::new(cdk10::Kind::HTLC, hash, Some(tags))
}

pub async fn generate_htlc_proofs(
    amount: cashu::Amount,
    locktime: TStamp,
    hash: &str,
    pk: cashu::PublicKey,
    keyset: &cashu::KeySet,
    keycl: &dyn KeysClient,
) -> Result<Vec<cashu::Proof>> {
    let amounts = amount.split();
    let secrets = std::iter::repeat_with(|| generate_htlc_secret(locktime, hash, pk))
        .map(cashu::secret::Secret::try_from)
        .take(amounts.len())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let premints = cashu::PreMintSecrets::from_secrets(keyset.id, amounts, secrets)?;
    let blinds = premints.blinded_messages();
    let signatures = keycl.sign(&blinds).await?;
    let proofs = unblind_signatures(premints, signatures.into_iter(), &keyset)?;
    Ok(proofs)
}
