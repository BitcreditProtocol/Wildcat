// ----- standard library imports
use std::{collections::HashMap, time::Duration};
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_common::cashu;
use bcr_wdc_utils::redis::DBConnConfig;
use redis::{AsyncCommands, RedisError, RedisResult};
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence,
};

// ----- end imports

const PROOF_KEY_PREFIX: &str = "proof";

fn proof_key(y: &cashu::PublicKey) -> String {
    format!("{}:{}", PROOF_KEY_PREFIX, y)
}

fn proof_to_state(proof: cashu::Proof) -> Result<cashu::ProofState> {
    let y = proof.y()?;
    let state = cashu::ProofState {
        y,
        state: cashu::State::Spent,
        witness: proof.witness,
    };
    Ok(state)
}

#[derive(Clone, Debug)]
pub struct Proofs<Conn> {
    pub(crate) conn: Conn,
}

impl Proofs<redis::aio::ConnectionManager> {
    pub async fn new(cfg: DBConnConfig) -> RedisResult<Self> {
        let client = redis::Client::open(cfg.url)?;
        let tout = Duration::from_secs(cfg.timeout_seconds);
        let cfg = redis::aio::ConnectionManagerConfig::default().set_connection_timeout(Some(tout));
        let manager = redis::aio::ConnectionManager::new_with_config(client, cfg).await?;
        Ok(Self { conn: manager })
    }
}

#[async_trait]
impl<Conn> persistence::ProofRepository for Proofs<Conn>
where
    Conn: redis::aio::ConnectionLike + Clone + Send + Sync,
{
    async fn insert(&self, tokens: Vec<cashu::Proof>) -> Result<()> {
        let double_spent_err = RedisError::from((
            redis::ErrorKind::UnexpectedReturnType,
            "One or more proofs already spent",
        ));
        let k_v = tokens
            .into_iter()
            .map(|item| {
                let item = proof_to_state(item)?;
                let key = proof_key(&item.y);
                let value =
                    serde_json::to_string(&item).map_err(|e| Error::ProofRepository(anyhow!(e)))?;
                Ok((key, value))
            })
            .collect::<Result<HashMap<String, String>>>()?;
        let keys: Vec<String> = k_v.keys().cloned().collect();
        let conn = self.conn.clone();
        let cloned_double_spent_err = double_spent_err.clone();
        let res = redis::aio::transaction_async(conn, &keys, move |mut conn, mut pipe| {
            let k_v_cloned = k_v.clone();
            let cloned_double_spent_err = double_spent_err.clone();
            async move {
                let keys: Vec<String> = k_v_cloned.keys().cloned().collect();
                for key in &keys {
                    let exist = conn.exists(key).await?;
                    if exist {
                        return Err(cloned_double_spent_err);
                    }
                }
                for (key, value) in k_v_cloned {
                    pipe.set(key, value).ignore();
                }
                pipe.exec_async(&mut conn).await?;
                Ok(Some(()))
            }
        })
        .await;
        match res {
            Ok(()) => Ok(()),
            Err(e) if e == cloned_double_spent_err => Err(Error::InvalidInput(String::from(
                "one or more proofs alrady spent",
            ))),
            Err(e) => Err(Error::ProofRepository(anyhow!(e))),
        }
    }

    async fn remove(&self, tokens: &[cashu::PublicKey]) -> Result<()> {
        let keys: Vec<String> = tokens.iter().map(proof_key).collect();
        let mut conn = self.conn.clone();
        let _: () = conn
            .del(keys)
            .await
            .map_err(|e| Error::ProofRepository(anyhow!(e)))?;

        Ok(())
    }

    async fn contains(&self, y: cashu::PublicKey) -> Result<Option<cashu::ProofState>> {
        let mut conn = self.conn.clone();
        let key = proof_key(&y);
        let value: Option<String> = conn
            .get(key)
            .await
            .map_err(|e| Error::ProofRepository(anyhow!(e)))?;
        let Some(jason) = value else {
            return Ok(None);
        };
        let state: cashu::ProofState =
            serde_json::from_str(&jason).map_err(|e| Error::ProofRepository(anyhow!(e)))?;
        Ok(Some(state))
    }
}
