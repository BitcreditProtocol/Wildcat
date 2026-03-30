// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::{
    cashu, client::core::Client as CoreClient, core::signature::unblind_ecash_signature,
    wallet::Token,
};
// ----- local imports
use crate::error::Result;

// ----- end imports

pub struct Service {
    pub crcore: Arc<CoreClient>,
}

#[derive(serde::Deserialize)]
pub enum IssueType {
    KeysetId(cashu::Id),
    Currency(cashu::CurrencyUnit),
}

#[derive(serde::Deserialize)]
pub struct MintRequest {
    pub amount: cashu::Amount,
    pub issue: IssueType,
    pub whoami: cashu::MintUrl,
}

pub async fn free_money(
    State(cntrl): State<Arc<Service>>,
    Json(request): Json<MintRequest>,
) -> Result<Json<String>> {
    tracing::debug!("Free money request");

    let kinfos = cntrl.crcore.list_keyset_info(Default::default()).await?;
    let token = match request.issue {
        IssueType::KeysetId(kid) => {
            let Some(kinfo) = kinfos.iter().find(|ks| ks.id == kid && ks.active) else {
                return Err(crate::error::Error::InvalidInput(format!(
                    "Unknown/Invalid keyset: {kid}"
                )));
            };
            let keys = cntrl.crcore.keys(kinfo.id).await?;
            mint(&cntrl.crcore, request.amount, keys, request.whoami).await?
        }
        IssueType::Currency(unit) => {
            let Some(kinfo) = kinfos.iter().find(|k| k.active && k.unit == unit) else {
                return Err(crate::error::Error::InvalidInput(format!(
                    "Unsupported currency unit: {unit}",
                )));
            };
            let keys = cntrl.crcore.keys(kinfo.id).await?;
            mint(&cntrl.crcore, request.amount, keys, request.whoami).await?
        }
    };
    Ok(Json(token.to_string()))
}

async fn mint(
    client: &CoreClient,
    amount: cashu::Amount,
    keys: cashu::KeySet,
    mint_url: cashu::MintUrl,
) -> Result<Token> {
    let premints =
        cashu::PreMintSecrets::random(keys.id, amount, &cashu::amount::SplitTarget::None)?;
    let blinded_messages = premints.blinded_messages();
    let signatures = client.sign(&blinded_messages).await?;
    let mut proofs = Vec::with_capacity(signatures.len());
    for (sig, pre) in signatures.into_iter().zip(premints.iter()) {
        // WARNING: due to a bug in `into_iter()` in cashu 0.13.1 we need to `iter()` and clone the secret
        // fixed in 0.14.0
        let proof = unblind_ecash_signature(&keys, pre.clone(), sig)?;
        proofs.push(proof);
    }
    Ok(Token::new_bitcr(mint_url, proofs, None, keys.unit.clone()))
}
