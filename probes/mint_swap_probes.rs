//! Probes for ticket #1071 against bcr-wdc-mint-service: persist before sign/stream, and a
//! failed store fails the request. Copied into the crate's tests/ by probes/run.
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex,
};

use async_trait::async_trait;
use bcr_common::{
    cashu::{self, ProofsMethods},
    client::admin::core::Client as CoreClient,
    core, core_tests, ecash,
    wire::{keys as wire_keys, swap as wire_swap},
};
use bcr_wdc_mint_service::{
    core::{
        clients::{ClowderClient, DummyClowderClient, PublicKeyOwner, TreasuryService},
        factory::KeysFactory,
        service::Service,
    },
    error::{Error, Result},
    persistence::{self, Repository, SignatureOwner, StoredCommitment, StoredSignature},
    test_utils,
};
use bcr_wdc_utils::{keys as keys_utils, signatures::test_utils as signatures_test};
use bitcoin::secp256k1::{schnorr, PublicKey};

type TStamp = time::OffsetDateTime;

// ------------------------------------------------------------------ backends

static DB_COUNTER: AtomicUsize = AtomicUsize::new(0);

async fn sqlx_repo() -> Arc<dyn Repository> {
    let base = std::env::var("PROBE_PG_URL")
        .expect("PROBE_PG_URL must point at a Postgres admin database (probes/run sets it)");
    let name = format!(
        "probe_{}_{}",
        std::process::id(),
        DB_COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    let admin = sqlx::PgPool::connect(&base).await.expect("connect admin");
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .expect("create db");
    let url = match base.rsplit_once('/') {
        Some((prefix, _)) => format!("{prefix}/{name}"),
        None => panic!("bad PROBE_PG_URL"),
    };
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect(&url)
        .await
        .expect("connect");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrate");
    Arc::new(persistence::sqlx::Repository::from_pool(pool))
}

async fn surreal_repo() -> Arc<dyn Repository> {
    let cfg = bcr_wdc_utils::surreal::DBConnConfig {
        connection: String::from("mem://"),
        namespace: String::from("probe"),
        database: String::from("probe"),
    };
    Arc::new(
        persistence::surreal::Repository::new(cfg)
            .await
            .expect("surreal"),
    )
}

async fn backend(name: &str) -> Arc<dyn Repository> {
    match name {
        "inmemory" => Arc::new(persistence::inmemory::Repository::default()),
        "surreal" => surreal_repo().await,
        "sqlx" => sqlx_repo().await,
        _ => unreachable!(),
    }
}

const BACKENDS: [&str; 3] = ["inmemory", "surreal", "sqlx"];

/// Runs `probe` once per backend and fails listing every backend that failed.
macro_rules! per_backend {
    ($probe:ident) => {{
        let mut failures = Vec::new();
        for name in BACKENDS {
            let handle = tokio::spawn(async move { $probe(name).await });
            if let Err(e) = handle.await {
                let msg = e
                    .try_into_panic()
                    .ok()
                    .and_then(|p| {
                        p.downcast_ref::<String>()
                            .cloned()
                            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    })
                    .unwrap_or_default();
                failures.push(format!("[{name}] {msg}"));
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }};
}

// ------------------------------------------------------------------ fault-injecting repository

#[derive(Default)]
struct Faults {
    commitment_store_errors: AtomicUsize,
    commitment_store_conflicts: AtomicUsize,
    swap_finalize_errors: AtomicUsize,
}

fn take(counter: &AtomicUsize) -> bool {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok()
}

struct FaultRepo {
    inner: Arc<dyn Repository>,
    faults: Arc<Faults>,
    log: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Repository for FaultRepo {
    async fn keys_store(&self, keys: keys_utils::MintKeysEntry) -> Result<()> {
        self.inner.keys_store(keys).await
    }
    async fn keys_info(&self, id: cashu::Id) -> Result<Option<ecash::MintKeySetInfo>> {
        self.inner.keys_info(id).await
    }
    async fn keys_load(&self, id: cashu::Id) -> Result<Option<ecash::MintKeySet>> {
        self.inner.keys_load(id).await
    }
    async fn keys_list_info(
        &self,
        currency: Option<cashu::CurrencyUnit>,
        min: Option<u64>,
        max: Option<u64>,
    ) -> Result<Vec<ecash::MintKeySetInfo>> {
        self.inner.keys_list_info(currency, min, max).await
    }
    async fn keys_infos_for_expiration_date(
        &self,
        expire: u64,
    ) -> Result<Vec<ecash::MintKeySetInfo>> {
        self.inner.keys_infos_for_expiration_date(expire).await
    }
    async fn signature_store(
        &self,
        y: cashu::PublicKey,
        signature: cashu::BlindSignature,
    ) -> Result<()> {
        self.inner.signature_store(y, signature).await
    }
    async fn signature_load(
        &self,
        blind: &cashu::BlindedMessage,
    ) -> Result<Option<cashu::BlindSignature>> {
        self.inner.signature_load(blind).await
    }
    async fn swap_finalize(
        &self,
        proofs: Vec<cashu::Proof>,
        signatures: Vec<StoredSignature>,
        commitment: schnorr::Signature,
    ) -> Result<()> {
        if take(&self.faults.swap_finalize_errors) {
            self.log.lock().unwrap().push("swap_finalize:err".into());
            return Err(Error::ProofRepository(anyhow::anyhow!("injected db outage")));
        }
        let r = self
            .inner
            .swap_finalize(proofs, signatures, commitment)
            .await;
        self.log
            .lock()
            .unwrap()
            .push(format!("swap_finalize:{}", if r.is_ok() { "ok" } else { "err" }));
        r
    }
    async fn proofs_insert(&self, tokens: Vec<cashu::Proof>) -> Result<()> {
        self.inner.proofs_insert(tokens).await
    }
    async fn proofs_remove(&self, tokens: &[cashu::PublicKey]) -> Result<()> {
        self.inner.proofs_remove(tokens).await
    }
    async fn proofs_contains(&self, y: cashu::PublicKey) -> Result<Option<cashu::ProofState>> {
        self.inner.proofs_contains(y).await
    }
    async fn commitment_store(
        &self,
        inputs: Vec<cashu::PublicKey>,
        outputs: Vec<cashu::PublicKey>,
        expiration: TStamp,
        wallet_key: cashu::PublicKey,
        commitment: schnorr::Signature,
        fp_digest: [u8; 32],
        signed: SignatureOwner,
    ) -> Result<()> {
        if take(&self.faults.commitment_store_errors) {
            return Err(Error::CommitmentRepository(anyhow::anyhow!(
                "injected db outage"
            )));
        }
        if take(&self.faults.commitment_store_conflicts) {
            return Err(Error::Conflict(String::from("injected conflict")));
        }
        self.inner
            .commitment_store(
                inputs, outputs, expiration, wallet_key, commitment, fp_digest, signed,
            )
            .await
    }
    async fn commitment_load(&self, signature: &schnorr::Signature) -> Result<StoredCommitment> {
        self.inner.commitment_load(signature).await
    }
    async fn commitment_find_by_input(
        &self,
        input: cashu::PublicKey,
    ) -> Result<Option<schnorr::Signature>> {
        self.inner.commitment_find_by_input(input).await
    }
    async fn commitment_contains_inputs(&self, inputs: &[cashu::PublicKey]) -> Result<bool> {
        self.inner.commitment_contains_inputs(inputs).await
    }
    async fn commitment_contains_outputs(&self, outputs: &[cashu::PublicKey]) -> Result<bool> {
        self.inner.commitment_contains_outputs(outputs).await
    }
    async fn commitment_delete(&self, commitment: schnorr::Signature) -> Result<()> {
        self.inner.commitment_delete(commitment).await
    }
    async fn commitment_clean_expired(&self, now: TStamp) -> Result<()> {
        self.inner.commitment_clean_expired(now).await
    }
    async fn ys_store(&self, inputs: Vec<cashu::PublicKey>, deadline: TStamp) -> Result<()> {
        self.inner.ys_store(inputs, deadline).await
    }
    async fn ys_contains(&self, inputs: &[cashu::PublicKey]) -> Result<Vec<bool>> {
        self.inner.ys_contains(inputs).await
    }
    async fn ys_clean_expired(&self, now: TStamp) -> Result<()> {
        self.inner.ys_clean_expired(now).await
    }
}

// ------------------------------------------------------------------ recording Clowder / treasury

#[derive(Clone, Debug)]
struct Signal {
    ys: Vec<cashu::PublicKey>,
    commitment: schnorr::Signature,
    signatures: Vec<cashu::BlindSignature>,
}

#[derive(Default)]
struct ClowderState {
    signals: Mutex<Vec<Signal>>,
    commits: AtomicUsize,
    violations: Mutex<Vec<String>>,
    fail_signals: AtomicUsize,
}

struct RecClowder {
    repo: Arc<dyn Repository>,
    state: Arc<ClowderState>,
    log: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ClowderClient for RecClowder {
    async fn mint_ebill(
        &self,
        _keyset_id: cashu::Id,
        _quote_id: uuid::Uuid,
        _amount: cashu::Amount,
        _bill_id: core::BillId,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        Ok(signatures)
    }
    async fn new_keyset(&self, _keyset: ecash::KeySet) -> Result<()> {
        Ok(())
    }
    async fn commit_to_swap(
        &self,
        request: wire_swap::SwapCommitmentRequest,
    ) -> Result<(String, schnorr::Signature)> {
        self.state.commits.fetch_add(1, Ordering::SeqCst);
        DummyClowderClient.commit_to_swap(request).await
    }
    async fn signal_swap_event(
        &self,
        inputs: Vec<cashu::Proof>,
        outputs: Vec<cashu::BlindedMessage>,
        _fees: Vec<cashu::BlindSignature>,
        commitment: schnorr::Signature,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<()> {
        self.log.lock().unwrap().push("signal".into());
        let ys = inputs.ys().unwrap();
        for y in &ys {
            let state = self.repo.proofs_contains(*y).await.unwrap();
            if !matches!(state, Some(ref s) if s.state == cashu::State::Spent) {
                self.state
                    .violations
                    .lock()
                    .unwrap()
                    .push(format!("Clowder streamed input {y} not persisted as spent: {state:?}"));
            }
        }
        for (blind, sig) in outputs.iter().zip(signatures.iter()) {
            let stored = self.repo.signature_load(blind).await.unwrap();
            if stored.as_ref() != Some(sig) {
                self.state.violations.lock().unwrap().push(format!(
                    "Clowder streamed signature for {} not persisted",
                    blind.blinded_secret
                ));
            }
        }
        if take(&self.state.fail_signals) {
            return Err(Error::ServiceUnavailable);
        }
        self.state.signals.lock().unwrap().push(Signal {
            ys,
            commitment,
            signatures,
        });
        Ok(())
    }
    async fn authenticate_attestation(
        &self,
        _alpha_id: &PublicKey,
        _inputs: &bcr_common::wire::attestation::AttestedFingerprints,
    ) -> Result<()> {
        Ok(())
    }
    async fn verify_pk(&self, _mint_pk: &PublicKey) -> Result<PublicKeyOwner> {
        Ok(PublicKeyOwner::Beta)
    }
}

#[derive(Default)]
struct RecTreasury {
    proofs: Mutex<Vec<cashu::Proof>>,
    fail: AtomicUsize,
}

#[async_trait]
impl TreasuryService for RecTreasury {
    async fn store_proofs(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        if take(&self.fail) {
            return Err(Error::Internal(String::from("injected treasury outage")));
        }
        self.proofs.lock().unwrap().extend(proofs);
        Ok(())
    }
}

// ------------------------------------------------------------------ harness

struct Harness {
    svc: Arc<Service>,
    repo: Arc<dyn Repository>,
    faults: Arc<Faults>,
    clowder: Arc<ClowderState>,
    log: Arc<Mutex<Vec<String>>>,
    treasury: Arc<RecTreasury>,
    keyset: ecash::MintKeySet,
}

impl Harness {
    async fn new(backend_name: &str) -> Self {
        let inner = backend(backend_name).await;
        Self::over(inner).await
    }

    async fn over(inner: Arc<dyn Repository>) -> Self {
        let faults = Arc::new(Faults::default());
        let log = Arc::new(Mutex::new(Vec::new()));
        let repo: Arc<dyn Repository> = Arc::new(FaultRepo {
            inner: inner.clone(),
            faults: faults.clone(),
            log: log.clone(),
        });
        let (mut kinfo, mut keyset) = core_tests::generate_random_ecash_keyset();
        kinfo.input_fee_ppk = 0;
        keyset.input_fee_ppk = 0;
        repo.keys_store(keys_utils::to_entry(kinfo, keyset.clone()))
            .await
            .unwrap();
        let clowder = Arc::new(ClowderState::default());
        let svc = Arc::new(service(repo.clone(), inner.clone(), clowder.clone(), log.clone()));
        Self {
            svc,
            repo,
            faults,
            clowder,
            log,
            treasury: Arc::default(),
            keyset,
        }
    }

    /// A second service over the same storage, as after a restart.
    fn restarted(&self) -> Arc<Service> {
        Arc::new(service(
            self.repo.clone(),
            self.repo.clone(),
            self.clowder.clone(),
            self.log.clone(),
        ))
    }

    fn proofs(&self, amounts: &[u64]) -> Vec<cashu::Proof> {
        let amounts: Vec<_> = amounts.iter().map(|a| cashu::Amount::from(*a)).collect();
        core_tests::generate_random_ecash_proofs(&self.keyset, &amounts)
    }

    fn blinds(&self, amounts: &[u64]) -> Vec<cashu::BlindedMessage> {
        let amounts: Vec<_> = amounts.iter().map(|a| cashu::Amount::from(*a)).collect();
        signatures_test::generate_blinds(self.keyset.id.into(), &amounts)
            .into_iter()
            .map(|g| g.0)
            .collect()
    }

    async fn commit(
        &self,
        proofs: &[cashu::Proof],
        blinds: &[cashu::BlindedMessage],
        expiry: u64,
        wallet: PublicKey,
        now: TStamp,
    ) -> Result<(String, schnorr::Signature)> {
        self.svc
            .commit_to_swap(request(proofs, blinds, expiry, wallet), now)
            .await
    }

    async fn swap(
        &self,
        proofs: &[cashu::Proof],
        blinds: &[cashu::BlindedMessage],
        commitment: schnorr::Signature,
        now: TStamp,
    ) -> Result<Vec<cashu::BlindSignature>> {
        self.svc
            .swap(
                self.treasury.as_ref(),
                proofs.to_vec(),
                blinds.to_vec(),
                commitment,
                now,
            )
            .await
    }

    async fn spent(&self, proofs: &[cashu::Proof]) -> Vec<bool> {
        let mut out = Vec::new();
        for y in proofs.to_vec().ys().unwrap() {
            let s = self.repo.proofs_contains(y).await.unwrap();
            out.push(matches!(s, Some(s) if s.state == cashu::State::Spent));
        }
        out
    }

    fn violations(&self) -> Vec<String> {
        self.clowder.violations.lock().unwrap().clone()
    }

    fn signals(&self) -> Vec<Signal> {
        self.clowder.signals.lock().unwrap().clone()
    }
}

fn service(
    repo: Arc<dyn Repository>,
    observed: Arc<dyn Repository>,
    clowder: Arc<ClowderState>,
    log: Arc<Mutex<Vec<String>>>,
) -> Service {
    Service {
        repository: repo,
        clowder: Box::new(RecClowder {
            repo: observed,
            state: clowder,
            log,
        }),
        keygen: KeysFactory::new(&[7u8; 32], bitcoin::bip32::DerivationPath::default()),
        min_keyset_fees_ppk: AtomicU64::new(0),
        max_expiry: time::Duration::hours(1),
        alpha_id: test_utils::mint_kp().public_key(),
        settle_window_deadline: TStamp::UNIX_EPOCH,
    }
}

fn request(
    proofs: &[cashu::Proof],
    blinds: &[cashu::BlindedMessage],
    expiry: u64,
    wallet: PublicKey,
) -> wire_swap::SwapCommitmentRequest {
    let fps: Vec<wire_keys::ProofFingerprint> = proofs
        .iter()
        .cloned()
        .map(wire_keys::ProofFingerprint::try_from)
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    wire_swap::SwapCommitmentRequest {
        inputs: test_utils::attested_fingerprints(fps),
        outputs: blinds.iter().cloned().map(From::from).collect(),
        expiry,
        wallet_key: wallet,
    }
}

fn now() -> TStamp {
    time::OffsetDateTime::now_utc()
}

fn in_secs(now: TStamp, secs: i64) -> u64 {
    (now + time::Duration::seconds(secs)).unix_timestamp() as u64
}

fn wallet() -> PublicKey {
    core::generate_random_keypair().public_key()
}

// ------------------------------------------------------------------ probes: commit

/// A commitment whose store fails (any error, not just a conflict) must fail the request:
/// no Clowder-signed commitment may come back for something that was never persisted.
async fn commit_store_failure_fails_request(name: &'static str) {
    for conflict in [false, true] {
        let h = Harness::new(name).await;
        let t = now();
        let proofs = h.proofs(&[8]);
        let blinds = h.blinds(&[8]);
        if conflict {
            h.faults.commitment_store_conflicts.store(1, Ordering::SeqCst);
        } else {
            h.faults.commitment_store_errors.store(1, Ordering::SeqCst);
        }
        let r = h.commit(&proofs, &blinds, in_secs(t, 120), wallet(), t).await;
        assert!(
            r.is_err(),
            "commit returned a signed commitment although its store failed (conflict={conflict}): {r:?}"
        );
    }
}
#[tokio::test]
async fn probe_commit_store_failure_fails_request() {
    per_backend!(commit_store_failure_fails_request);
}

/// Every commitment the service hands out must be persisted and swappable.
async fn returned_commitment_is_persisted(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[8, 4]);
    let blinds = h.blinds(&[8, 4]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    let stored = h.repo.commitment_load(&c).await;
    assert!(stored.is_ok(), "returned commitment not loadable");
    h.swap(&proofs, &blinds, c, t).await.expect("swap");
}
#[tokio::test]
async fn probe_returned_commitment_is_persisted() {
    per_backend!(returned_commitment_is_persisted);
}

/// A failed request must not leave its inputs locked: after a transient store failure, the
/// same wallet retrying the same request gets a commitment and can swap.
async fn commit_retry_after_store_failure(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[8]);
    let blinds = h.blinds(&[8]);
    let w = wallet();
    let expiry = in_secs(t, 600);
    h.faults.commitment_store_errors.store(1, Ordering::SeqCst);
    assert!(h.commit(&proofs, &blinds, expiry, w, t).await.is_err());
    let r = h.commit(&proofs, &blinds, expiry, w, t).await;
    let (_, c) = r.unwrap_or_else(|e| {
        panic!("identical retry after a failed store is refused until the 600s expiry: {e}")
    });
    h.swap(&proofs, &blinds, c, t).await.expect("swap");
}
#[tokio::test]
async fn probe_commit_retry_after_store_failure() {
    per_backend!(commit_retry_after_store_failure);
}

/// Two different commitments over the same or overlapping inputs: the second fails, the
/// non-overlapping input of the failed request stays usable, and the first still swaps.
async fn overlapping_commit_rejected(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let ab = h.proofs(&[8, 4]);
    let c_only = h.proofs(&[2]);
    let first_out = h.blinds(&[8, 4]);
    let (_, first) = h
        .commit(&ab, &first_out, in_secs(t, 120), wallet(), t)
        .await
        .expect("first commit");
    let same = h
        .commit(&ab, &h.blinds(&[8, 4]), in_secs(t, 120), wallet(), t)
        .await;
    assert!(same.is_err(), "second commit over same inputs succeeded");
    let same_wallet_other_outputs = h
        .commit(&ab, &h.blinds(&[4, 8]), in_secs(t, 120), wallet(), t)
        .await;
    assert!(same_wallet_other_outputs.is_err());
    let bc = vec![ab[1].clone(), c_only[0].clone()];
    let overlap = h
        .commit(&bc, &h.blinds(&[4, 2]), in_secs(t, 120), wallet(), t)
        .await;
    assert!(overlap.is_err(), "commit over overlapping inputs succeeded");
    let c_alone = h
        .commit(&c_only, &h.blinds(&[2]), in_secs(t, 120), wallet(), t)
        .await;
    assert!(
        c_alone.is_ok(),
        "input of a rejected overlapping request left locked: {c_alone:?}"
    );
    h.swap(&ab, &first_out, first, t).await.expect("first swaps");
}
#[tokio::test]
async fn probe_overlapping_commit_rejected() {
    per_backend!(overlapping_commit_rejected);
}

/// Many concurrent commits with distinct outputs over the same inputs: at most one wins, and
/// whatever wins is persisted.
async fn concurrent_commits(name: &'static str) {
    let h = Arc::new(Harness::new(name).await);
    let t = now();
    let proofs = h.proofs(&[8, 2]);
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let h = h.clone();
        let proofs = proofs.clone();
        let blinds = h.blinds(&[8, 2]);
        tasks.push(tokio::spawn(async move {
            let r = h.commit(&proofs, &blinds, in_secs(t, 120), wallet(), t).await;
            (r, blinds)
        }));
    }
    let mut wins = Vec::new();
    for task in tasks {
        let (r, blinds) = task.await.unwrap();
        if let Ok((_, c)) = r {
            wins.push((c, blinds));
        }
    }
    assert!(wins.len() <= 1, "{} concurrent commits over the same inputs succeeded", wins.len());
    let mut swapped = 0;
    for (c, blinds) in wins {
        if h.swap(&proofs, &blinds, c, t).await.is_ok() {
            swapped += 1;
        }
    }
    assert!(swapped <= 1, "{swapped} swaps spent the same inputs");
}
#[tokio::test]
async fn probe_concurrent_commits() {
    per_backend!(concurrent_commits);
}

/// An identical retry returns the original commitment, also when the requested expiry is
/// beyond max_expiry (the service caps it) and the retry comes a moment later.
async fn identical_retry_capped_expiry(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[8]);
    let blinds = h.blinds(&[8]);
    let w = wallet();
    let expiry = in_secs(t, 10 * 3600);
    let (_, first) = h.commit(&proofs, &blinds, expiry, w, t).await.expect("commit");
    let later = t + time::Duration::milliseconds(1500);
    let retry = h.commit(&proofs, &blinds, expiry, w, later).await;
    match retry {
        Ok((_, c)) => assert_eq!(c, first, "retry returned a different commitment"),
        Err(e) => panic!("identical retry 1.5s later with expiry > max_expiry refused: {e}"),
    }
}
#[tokio::test]
async fn probe_identical_retry_capped_expiry() {
    per_backend!(identical_retry_capped_expiry);
}

/// The identical retry also holds for a plain expiry on every backend.
async fn identical_retry(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[8, 4, 1]);
    let blinds = h.blinds(&[8, 4, 1]);
    let w = wallet();
    let expiry = in_secs(t, 300);
    let (_, first) = h.commit(&proofs, &blinds, expiry, w, t).await.expect("commit");
    let mut reordered = proofs.clone();
    reordered.reverse();
    let (_, again) = h
        .commit(&proofs, &blinds, expiry, w, t + time::Duration::seconds(1))
        .await
        .expect("retry");
    assert_eq!(again, first);
    let other_wallet = h.commit(&proofs, &blinds, expiry, wallet(), t).await;
    assert!(other_wallet.is_err(), "another wallet key got the commitment back");
    let other_expiry = h.commit(&proofs, &blinds, expiry + 1, w, t).await;
    assert!(other_expiry.is_err(), "a different expiry got a commitment");
    h.swap(&proofs, &blinds, first, t).await.expect("swap");
    let after = h.commit(&proofs, &blinds, expiry, w, t).await;
    assert!(after.is_err(), "commit over spent inputs succeeded: {after:?}");
    let _ = reordered;
}
#[tokio::test]
async fn probe_identical_retry() {
    per_backend!(identical_retry);
}

/// Once a commitment expires its inputs are free for a new commitment, and the old one can
/// no longer swap.
async fn commit_after_expiry(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[8]);
    let a_out = h.blinds(&[8]);
    let (_, a) = h
        .commit(&proofs, &a_out, in_secs(t, 60), wallet(), t)
        .await
        .expect("commit A");
    let later = t + time::Duration::seconds(180);
    let b_out = h.blinds(&[8]);
    let b = h
        .commit(&proofs, &b_out, in_secs(later, 60), wallet(), later)
        .await;
    let (_, b) = b.unwrap_or_else(|e| panic!("inputs stay locked after commitment A expired: {e}"));
    assert!(h.swap(&proofs, &a_out, a, later).await.is_err(), "expired A swapped");
    h.swap(&proofs, &b_out, b, later).await.expect("B swaps");
}
#[tokio::test]
async fn probe_commit_after_expiry() {
    per_backend!(commit_after_expiry);
}

/// An admin reservation and a swap commitment can never both hold the same input.
async fn admin_reserve_vs_commit(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[8]);
    h.svc
        .reserve(proofs.ys().unwrap(), t + time::Duration::seconds(600))
        .await
        .expect("reserve");
    let r = h
        .commit(&proofs, &h.blinds(&[8]), in_secs(t, 120), wallet(), t)
        .await;
    assert!(r.is_err(), "commit over admin-reserved inputs succeeded");

    let proofs = h.proofs(&[4]);
    h.commit(&proofs, &h.blinds(&[4]), in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    let r = h
        .svc
        .reserve(proofs.ys().unwrap(), t + time::Duration::seconds(600))
        .await;
    assert!(r.is_err(), "admin reserve over committed inputs succeeded");
}
#[tokio::test]
async fn probe_admin_reserve_vs_commit() {
    per_backend!(admin_reserve_vs_commit);
}

// ------------------------------------------------------------------ probes: swap

/// Clowder only ever hears of a swap whose inputs and signatures are already stored; a
/// failed store fails the request and Clowder hears nothing.
async fn swap_persist_before_stream(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[16]);
    let blinds = h.blinds(&[8, 4, 2]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    h.faults.swap_finalize_errors.store(1, Ordering::SeqCst);
    let r = h.swap(&proofs, &blinds, c, t).await;
    assert!(r.is_err(), "swap succeeded although its store failed");
    assert!(h.signals().is_empty(), "Clowder notified of an unpersisted swap");
    assert_eq!(h.spent(&proofs).await, vec![false]);
    let sigs = h.swap(&proofs, &blinds, c, t).await.expect("swap after outage");
    assert!(h.violations().is_empty(), "{:?}", h.violations());
    assert_eq!(h.signals().len(), 1);
    assert_eq!(h.signals()[0].signatures, sigs);
    let log = h.log.lock().unwrap().clone();
    let fin = log.iter().position(|l| l == "swap_finalize:ok");
    let sig = log.iter().position(|l| l == "signal");
    assert!(fin.is_some() && fin < sig, "order: {log:?}");
}
#[tokio::test]
async fn probe_swap_persist_before_stream() {
    per_backend!(swap_persist_before_stream);
}

/// Many concurrent swaps of one commitment: every success returns the same signatures and
/// Clowder never sees the inputs under two commitments or two signature sets.
async fn concurrent_swaps(name: &'static str) {
    let h = Arc::new(Harness::new(name).await);
    let t = now();
    let proofs = h.proofs(&[8, 4]);
    let blinds = h.blinds(&[8, 4]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    let mut tasks = Vec::new();
    for _ in 0..12 {
        let (h, p, b) = (h.clone(), proofs.clone(), blinds.clone());
        tasks.push(tokio::spawn(async move { h.swap(&p, &b, c, t).await }));
    }
    let mut oks = Vec::new();
    for task in tasks {
        if let Ok(s) = task.await.unwrap() {
            oks.push(s);
        }
    }
    assert!(!oks.is_empty(), "no concurrent swap succeeded");
    let finalized = h
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|l| *l == "swap_finalize:ok")
        .count();
    let distinct_c = oks
        .iter()
        .map(|s| s.iter().map(|b| b.c).collect::<Vec<_>>())
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert!(
        finalized <= 1 && oks.windows(2).all(|w| w[0] == w[1]),
        "{} of 12 concurrent swaps succeeded; swap_finalize succeeded {finalized} times; \
         {distinct_c} distinct C values; {} Clowder signals",
        oks.len(),
        h.signals().len()
    );
    assert!(h.violations().is_empty(), "{:?}", h.violations());
    let signals = h.signals();
    assert!(signals.iter().all(|s| s.commitment == c && s.signatures == oks[0]));
    assert_eq!(h.spent(&proofs).await, vec![true, true]);
}
#[tokio::test]
async fn probe_concurrent_swaps() {
    per_backend!(concurrent_swaps);
}

/// Clowder must never hear the same inputs under a commitment other than the one that spent
/// them: replaying a finalized swap with a commitment that was never issued must fail
/// without a signal.
async fn replay_with_foreign_commitment(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[8]);
    let blinds = h.blinds(&[8]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    h.swap(&proofs, &blinds, c, t).await.expect("swap");
    let forged = schnorr::Signature::from_slice(&[0x5a; 64]).unwrap();
    let r = h.swap(&proofs, &blinds, forged, t).await;
    let streamed: Vec<_> = h
        .signals()
        .iter()
        .map(|s| s.commitment)
        .filter(|sc| *sc != c)
        .collect();
    assert!(
        streamed.is_empty(),
        "Clowder was streamed the spent inputs under a never-issued commitment {streamed:?} (swap returned {})",
        if r.is_ok() { "Ok" } else { "Err" }
    );
    assert!(r.is_err(), "swap with a never-issued commitment returned signatures");
}
#[tokio::test]
async fn probe_replay_with_foreign_commitment() {
    per_backend!(replay_with_foreign_commitment);
}

/// When Clowder is unreachable after the swap persisted, the request fails. There is no safe
/// way left to tell that a retry with the same commitment is this same swap rather than a
/// forged commitment over someone else's already-spent inputs (see
/// `probe_replay_with_foreign_commitment`), so the retry (also after a restart) fails too,
/// even though the signatures already exist in storage: exactly one Clowder signal is ever
/// sent for this swap.
async fn stream_failure_then_retry(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[16]);
    let blinds = h.blinds(&[8, 2]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    h.clowder.fail_signals.store(1, Ordering::SeqCst);
    let first = h.swap(&proofs, &blinds, c, t).await;
    assert!(first.is_err(), "swap reported success though Clowder was not told");
    let restarted = h.restarted();
    let retry = restarted
        .swap(h.treasury.as_ref(), proofs.clone(), blinds.clone(), c, t)
        .await;
    assert!(
        retry.is_err(),
        "retry after restart replayed a finalized swap instead of failing"
    );
    assert_eq!(h.signals().len(), 0, "Clowder was never successfully told");
}
#[tokio::test]
async fn probe_stream_failure_then_retry() {
    per_backend!(stream_failure_then_retry);
}

/// A treasury outage after the swap persisted: the request fails, and so does a retry with the
/// same commitment, for the same reason as `stream_failure_then_retry`. The fee proofs already
/// signed are not re-sent to the treasury either, since the retry never reaches that step.
async fn treasury_failure_then_retry(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[16]);
    let blinds = h.blinds(&[8]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    h.treasury.fail.store(1, Ordering::SeqCst);
    assert!(h.swap(&proofs, &blinds, c, t).await.is_err());
    let retry = h.swap(&proofs, &blinds, c, t).await;
    assert!(retry.is_err(), "retry replayed a finalized swap instead of failing");
    assert_eq!(h.signals().len(), 0, "Clowder signals");
}
#[tokio::test]
async fn probe_treasury_failure_then_retry() {
    per_backend!(treasury_failure_then_retry);
}

/// Finding #1 (critical): a retry of a finalized swap must not be able to smuggle a
/// commitment the mint never issued through to Clowder. Even when the inputs are spent and the
/// outputs already carry a signature (an honest retry would look identical), a forged
/// commitment over them is rejected outright rather than replayed.
async fn finalized_swap_retry_never_signals_twice(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[1]);
    let blinds = h.blinds(&[1]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    h.swap(&proofs, &blinds, c, t).await.expect("swap");
    assert_eq!(h.signals().len(), 1, "one Clowder signal for the original swap");
    for _ in 0..8 {
        let _ = h.swap(&proofs, &blinds, c, t).await;
    }
    assert_eq!(
        h.signals().len(),
        1,
        "every retry of a finalized swap must not re-signal Clowder"
    );
}
#[tokio::test]
async fn probe_finalized_swap_retry_never_signals_twice() {
    per_backend!(finalized_swap_retry_never_signals_twice);
}

// ------------------------------------------------------------------ probes: over HTTP

async fn http_setup() -> (
    axum_test::TestServer,
    CoreClient,
    ecash::MintKeySet,
) {
    let (info, keyset) = core_tests::generate_random_ecash_keyset();
    let (server, _) =
        test_utils::build_test_server(Some(keys_utils::to_entry(info, keyset.clone()))).await;
    let url = server.server_address().expect("address");
    (server, CoreClient::new(url), keyset)
}

fn http_blinds(keyset: &ecash::MintKeySet, amounts: &[u64]) -> Vec<ecash::BlindedMessage> {
    let amounts: Vec<_> = amounts.iter().map(|a| cashu::Amount::from(*a)).collect();
    signatures_test::generate_blinds(keyset.id.into(), &amounts)
        .into_iter()
        .map(|g| ecash::BlindedMessage::from(g.0))
        .collect()
}

/// Over HTTP, concurrent commits with different outputs over the same inputs: at most one
/// commitment comes back, and at most one swap succeeds; concurrent swaps of the winner
/// return identical signatures.
#[tokio::test]
async fn probe_http_concurrent_commit_and_swap() {
    let (_server, client, keyset) = http_setup().await;
    let client = Arc::new(client);
    let c_proofs = core_tests::generate_random_ecash_proofs(&keyset, &[cashu::Amount::from(8u64)]);
    let fps: Vec<wire_keys::ProofFingerprint> = c_proofs
        .iter()
        .cloned()
        .map(wire_keys::ProofFingerprint::try_from)
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    let mint_pk = test_utils::mint_kp().public_key();
    let expiry = in_secs(now(), 120);
    let mut tasks = Vec::new();
    for _ in 0..12 {
        let client = client.clone();
        let fps = fps.clone();
        let blinds = http_blinds(&keyset, &[8]);
        tasks.push(tokio::spawn(async move {
            let r = client
                .commit_swap(
                    fps.clone(),
                    blinds.clone(),
                    expiry,
                    wallet(),
                    mint_pk,
                    test_utils::dummy_attestation_for(&fps),
                )
                .await;
            (r.map(|(_, c)| c), blinds)
        }));
    }
    let mut wins = Vec::new();
    for t in tasks {
        let (r, blinds) = t.await.unwrap();
        if let Ok(c) = r {
            wins.push((c, blinds));
        }
    }
    assert!(wins.len() <= 1, "{} HTTP commits over the same inputs succeeded", wins.len());
    let proofs: Vec<_> = c_proofs.iter().cloned().map(ecash::Proof::from).collect();
    if let Some((c, blinds)) = wins.pop() {
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let (client, p, b) = (client.clone(), proofs.clone(), blinds.clone());
            tasks.push(tokio::spawn(async move { client.swap(p, b, c).await }));
        }
        let mut oks = Vec::new();
        for t in tasks {
            if let Ok(s) = t.await.unwrap() {
                oks.push(s);
            }
        }
        assert!(!oks.is_empty(), "no HTTP swap succeeded");
        assert!(oks.windows(2).all(|w| w[0] == w[1]), "HTTP swaps returned different signatures");
    }
}

/// Over HTTP: after a swap finalized, posting the same proofs and outputs with a commitment the
/// mint never issued must be refused, not answered with the stored signatures.
#[tokio::test]
async fn probe_http_replay_with_foreign_commitment() {
    let (_server, client, keyset) = http_setup().await;
    let c_proofs = core_tests::generate_random_ecash_proofs(&keyset, &[cashu::Amount::from(8u64)]);
    let fps: Vec<wire_keys::ProofFingerprint> = c_proofs
        .iter()
        .cloned()
        .map(wire_keys::ProofFingerprint::try_from)
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    let blinds = http_blinds(&keyset, &[8]);
    let (_, c) = client
        .commit_swap(
            fps.clone(),
            blinds.clone(),
            in_secs(now(), 120),
            wallet(),
            test_utils::mint_kp().public_key(),
            test_utils::dummy_attestation_for(&fps),
        )
        .await
        .expect("commit");
    let proofs: Vec<_> = c_proofs.iter().cloned().map(ecash::Proof::from).collect();
    client.swap(proofs.clone(), blinds.clone(), c).await.expect("swap");
    let forged = schnorr::Signature::from_slice(&[0x5a; 64]).unwrap();
    let r = client.swap(proofs, blinds, forged).await;
    assert!(r.is_err(), "POST swap with a never-issued commitment answered 200 with signatures");
}

// ------------------------------------------------------------------ probes: persistence (sqlx only)

/// Finding #2 (high, sqlx/Postgres only): a melt or admin reservation holds a `y` with its own
/// deadline. A swap commitment over that same input must not claim the row: only a reservation
/// made with this commitment's own expiry (the one `commit_to_swap_inner` takes just before
/// calling this) may be claimed.
#[tokio::test]
async fn probe_commitment_store_cannot_steal_other_reservation() {
    let repo = sqlx_repo().await;
    let t = now();
    let y = core_tests::generate_random_ecash_proofs(
        &core_tests::generate_random_ecash_keyset().1,
        &[cashu::Amount::from(8u64)],
    )
    .ys()
    .unwrap()[0];
    let melt_deadline = t + time::Duration::seconds(600);
    repo.ys_store(vec![y], melt_deadline).await.expect("melt/admin reservation");
    let signature = schnorr::Signature::from_slice(&[0x11; 64]).unwrap();
    let outcome = repo
        .commitment_store(
            vec![y],
            vec![],
            t + time::Duration::seconds(120),
            wallet().into(),
            signature,
            [0u8; 32],
            SignatureOwner::Unsigned,
        )
        .await;
    assert!(
        outcome.is_err(),
        "a swap commitment took over a reservation it never made"
    );
    assert!(
        repo.ys_contains(&[y]).await.unwrap()[0],
        "the melt/admin reservation must still hold the input"
    );
}
