// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::{
    client::keys::{Client as KeysClient, Result as KeysResult},
    core::signature::unblind_ecash_signature,
};
use bcr_wallet_lib::wallet::Token;
use cdk::wallet::{HttpClient, MintConnector};
use futures::future::JoinAll;
// ----- local imports
use crate::error::Result;

// ----- end imports

pub struct Service {
    pub crkeys: KeysClient,
    pub dbmint: HttpClient,
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

    let cashu::KeysetResponse { keysets } = cntrl.dbmint.get_mint_keysets().await?;
    let kinfos = cntrl.crkeys.list_keyset_info().await?;

    let token = match request.issue {
        IssueType::KeysetId(kid) => {
            if let Some(kinfo) = keysets.iter().find(|ks| ks.id == kid && ks.active) {
                let keys = cntrl.dbmint.get_mint_keyset(kinfo.id).await?;
                mint_debit(&cntrl.dbmint, request.amount, keys, request.whoami).await?
            } else if let Some(kinfo) = kinfos.iter().find(|ks| ks.id == kid && ks.active) {
                let keys = cntrl.crkeys.keys(kinfo.id).await?;
                mint_credit(&cntrl.crkeys, request.amount, keys, request.whoami).await?
            } else {
                return Err(crate::error::Error::InvalidInput(format!(
                    "Unknown/Invalid keyset: {kid}"
                )));
            }
        }
        IssueType::Currency(unit) => {
            if let Some(kinfo) = keysets.iter().find(|k| k.active && k.unit == unit) {
                let keys = cntrl.dbmint.get_mint_keyset(kinfo.id).await?;
                mint_debit(&cntrl.dbmint, request.amount, keys, request.whoami).await?
            } else if let Some(kinfo) = kinfos.iter().find(|k| k.active && k.unit == unit) {
                let keys = cntrl.crkeys.keys(kinfo.id).await?;
                mint_credit(&cntrl.crkeys, request.amount, keys, request.whoami).await?
            } else {
                return Err(crate::error::Error::InvalidInput(format!(
                    "Unsupported currency unit: {unit}",
                )));
            }
        }
    };
    Ok(Json(token.to_string()))
}

async fn mint_credit(
    client: &KeysClient,
    amount: cashu::Amount,
    keys: cashu::KeySet,
    mint_url: cashu::MintUrl,
) -> Result<Token> {
    let premints =
        cashu::PreMintSecrets::random(keys.id, amount, &cashu::amount::SplitTarget::None)?;
    let blinded_messages = premints.blinded_messages();
    let joined: JoinAll<_> = blinded_messages
        .iter()
        .map(|blind| client.sign(blind))
        .collect();
    let signatures: Vec<cashu::BlindSignature> =
        joined.await.into_iter().collect::<KeysResult<_>>()?;
    let mut proofs = Vec::with_capacity(signatures.len());
    for (sig, pre) in signatures.into_iter().zip(premints.into_iter()) {
        let proof = unblind_ecash_signature(&keys, pre, sig)?;
        proofs.push(proof);
    }
    Ok(Token::new_bitcr(mint_url, proofs, None, keys.unit.clone()))
}

async fn mint_debit(
    client: &HttpClient,
    amount: cashu::Amount,
    keys: cashu::KeySet,
    mint_url: cashu::MintUrl,
) -> Result<Token> {
    let kp = secp256k1::Keypair::new(secp256k1::global::SECP256K1, &mut rand::thread_rng());
    let pubkey = cashu::PublicKey::from(kp.public_key());
    let request = cashu::MintQuoteBolt11Request {
        amount,
        unit: keys.unit.clone(),
        description: Some(String::from("it's me, Mario")),
        pubkey: Some(pubkey),
    };
    let quote = client.post_mint_quote(request).await?;
    let premints =
        cashu::PreMintSecrets::random(keys.id, amount, &cashu::amount::SplitTarget::None)?;
    let mut request = cashu::MintRequest {
        quote: quote.quote,
        outputs: premints.blinded_messages(),
        signature: None,
    };
    let sk = cashu::SecretKey::from(kp.secret_key());
    request.sign(sk)?;
    let response = client.post_mint(request).await?;
    let mut proofs = Vec::with_capacity(response.signatures.len());
    // WARNING: due to a bug in `into_iter()` in cashu 0.13.1 we need to `iter()` and clone the secret
    // fixed in 0.14.0
    for (sig, pre) in response.signatures.into_iter().zip(premints.iter()) {
        let proof = unblind_ecash_signature(&keys, pre.clone(), sig)?;
        proofs.push(proof);
    }
    Ok(Token::new_cashu(mint_url, proofs, None, keys.unit.clone()))
}
