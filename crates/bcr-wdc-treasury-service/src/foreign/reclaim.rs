// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bcr_common::cashu::{self, ProofsMethods};
use bcr_wdc_utils::{routine::Routine, TStamp};
use tracing::{info, warn};
// ----- local imports
use crate::foreign::{ClowderClient, KeysClient, OnlineRepository};
// ----- end imports

/// Takes back exchange eCash whose locktime has passed without the recipient ever
/// unlocking it. Only this mint's refund key can spend it now, and the collateral
/// it was issued against was never claimed, so retiring it is what keeps issuance
/// backed.
pub struct Handler {
    pub online: Arc<dyn OnlineRepository>,
    pub keys: Arc<dyn KeysClient>,
    pub clowder: Arc<dyn ClowderClient>,
}

#[async_trait]
impl Routine for Handler {
    async fn run_task(&self, now: TStamp) -> AnyResult<Option<std::time::Duration>> {
        let expired = self.online.list_expired_issued(now).await?;
        if expired.is_empty() {
            return Ok(None);
        }
        let ys: Vec<cashu::PublicKey> = expired
            .iter()
            .map(|proof| proof.y())
            .collect::<Result<_, _>>()?;
        let states = self.keys.proof_states(ys.clone()).await?;

        // A burn is all-or-nothing, so anything not unspent would take the reclaimable
        // proofs down with it. Spent means the recipient unlocked in time: settled.
        let mut burnable = Vec::with_capacity(expired.len());
        let mut burnable_ys = Vec::with_capacity(expired.len());
        let mut settled: Vec<cashu::PublicKey> = Vec::new();
        for (proof, y) in expired.into_iter().zip(ys) {
            match states.get(&y) {
                Some(cashu::State::Unspent) => {
                    burnable.push(proof);
                    burnable_ys.push(y);
                }
                Some(cashu::State::Spent) => settled.push(y),
                _ => (),
            }
        }

        if !burnable.is_empty() {
            let amount = burnable.total_amount().unwrap_or_default();
            // The core burns unconditionally: the expired locktime is the only gate.
            match self.keys.burn(burnable.clone()).await {
                Ok(()) => {
                    // The chain entry is what makes this mint's supply match what it
                    // can honour.
                    if let Err(e) = self.clowder.signal_burn_event(burnable).await {
                        warn!("Burned {amount} locally but could not record it on chain: {e}");
                    }
                    settled.extend(burnable_ys);
                    info!("Burned {amount} of expired exchange eCash");
                }
                Err(e) => warn!("Burn of {amount} deferred: {e}"),
            }
        }
        if !settled.is_empty() {
            self.online.remove_issued(&settled).await?;
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foreign::{MockClowderClient, MockKeysClient, MockOnlineRepository};
    use bcr_common::core_tests;
    use mockall::predicate::eq;

    fn proofs(n: usize) -> Vec<cashu::Proof> {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        core_tests::generate_random_ecash_proofs(&keyset, &vec![cashu::Amount::from(8u64); n])
    }

    fn states(
        proofs: &[cashu::Proof],
        states: &[cashu::State],
    ) -> std::collections::HashMap<cashu::PublicKey, cashu::State> {
        proofs
            .iter()
            .zip(states)
            .map(|(p, s)| (p.y().unwrap(), *s))
            .collect()
    }

    // One already-unlocked proof must not wedge the reclaimable ones, round after round.
    #[tokio::test]
    async fn spent_proofs_are_settled_not_burned() {
        let expired = proofs(2);
        let (spent, unspent) = (expired[0].clone(), expired[1].clone());
        let mut online = MockOnlineRepository::new();
        let mut keys = MockKeysClient::new();
        let mut clowder = MockClowderClient::new();

        let listed = expired.clone();
        online
            .expect_list_expired_issued()
            .times(1)
            .returning(move |_| Ok(listed.clone()));
        let reported = states(&expired, &[cashu::State::Spent, cashu::State::Unspent]);
        keys.expect_proof_states()
            .times(1)
            .returning(move |_| Ok(reported.clone()));
        keys.expect_burn()
            .with(eq(vec![unspent.clone()]))
            .times(1)
            .returning(|_| Ok(()));
        clowder
            .expect_signal_burn_event()
            .times(1)
            .returning(|_| Ok(()));
        // Both leave the table: one reclaimed, one settled.
        let mut removed = vec![spent.y().unwrap(), unspent.y().unwrap()];
        removed.sort();
        online
            .expect_remove_issued()
            .withf(move |ys| {
                let mut ys = ys.to_vec();
                ys.sort();
                ys == removed
            })
            .times(1)
            .returning(|_| Ok(()));

        let handler = Handler {
            online: Arc::new(online),
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
        };
        handler.run_task(chrono::Utc::now()).await.unwrap();
    }

    // Nothing to reclaim: no burn attempted, rows still go.
    #[tokio::test]
    async fn all_spent_burns_nothing() {
        let expired = proofs(2);
        let mut online = MockOnlineRepository::new();
        let mut keys = MockKeysClient::new();
        let clowder = MockClowderClient::new();

        let listed = expired.clone();
        online
            .expect_list_expired_issued()
            .times(1)
            .returning(move |_| Ok(listed.clone()));
        let reported = states(&expired, &[cashu::State::Spent, cashu::State::Spent]);
        keys.expect_proof_states()
            .times(1)
            .returning(move |_| Ok(reported.clone()));
        keys.expect_burn().never();
        online.expect_remove_issued().times(1).returning(|_| Ok(()));

        let handler = Handler {
            online: Arc::new(online),
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
        };
        handler.run_task(chrono::Utc::now()).await.unwrap();
    }

    // A proof mid-transaction is neither burned nor forgotten.
    #[tokio::test]
    async fn in_flight_proofs_are_left_for_the_next_round() {
        let expired = proofs(1);
        let mut online = MockOnlineRepository::new();
        let mut keys = MockKeysClient::new();
        let clowder = MockClowderClient::new();

        let listed = expired.clone();
        online
            .expect_list_expired_issued()
            .times(1)
            .returning(move |_| Ok(listed.clone()));
        let reported = states(&expired, &[cashu::State::Pending]);
        keys.expect_proof_states()
            .times(1)
            .returning(move |_| Ok(reported.clone()));
        keys.expect_burn().never();
        online.expect_remove_issued().never();

        let handler = Handler {
            online: Arc::new(online),
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
        };
        handler.run_task(chrono::Utc::now()).await.unwrap();
    }

    // A failed burn keeps its rows, so the value stays reclaimable.
    #[tokio::test]
    async fn failed_burn_keeps_the_rows() {
        let expired = proofs(1);
        let mut online = MockOnlineRepository::new();
        let mut keys = MockKeysClient::new();
        let clowder = MockClowderClient::new();

        let listed = expired.clone();
        online
            .expect_list_expired_issued()
            .times(1)
            .returning(move |_| Ok(listed.clone()));
        let reported = states(&expired, &[cashu::State::Unspent]);
        keys.expect_proof_states()
            .times(1)
            .returning(move |_| Ok(reported.clone()));
        keys.expect_burn()
            .times(1)
            .returning(|_| Err(crate::error::Error::Internal(String::from("core down"))));
        online.expect_remove_issued().never();

        let handler = Handler {
            online: Arc::new(online),
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
        };
        handler.run_task(chrono::Utc::now()).await.unwrap();
    }
}
