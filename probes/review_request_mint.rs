//! Review probes (request lens) for ticket #1071 against bcr-wdc-mint-service. Harness copied
//! from probes/mint_swap_probes.rs. Run by copying into crates/bcr-wdc-mint-service/tests/.
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
    keys_store_errors: AtomicUsize,
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
        if take(&self.faults.keys_store_errors) {
            return Err(Error::KeysRepository(anyhow::anyhow!("injected db outage")));
        }
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
    fail_commits: AtomicUsize,
    new_keysets: AtomicUsize,
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
        self.state.new_keysets.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn commit_to_swap(
        &self,
        request: wire_swap::SwapCommitmentRequest,
    ) -> Result<(String, schnorr::Signature)> {
        self.state.commits.fetch_add(1, Ordering::SeqCst);
        if take(&self.state.fail_commits) {
            return Err(Error::ServiceUnavailable);
        }
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

// ------------------------------------------------------------------ review probes (request lens)

use bcr_common::core::signature as core_signature;

fn mint_xonly() -> bitcoin::secp256k1::XOnlyPublicKey {
    test_utils::mint_kp().x_only_public_key().0
}

/// Every Ok commit response is a pair Clowder signed: the commitment verifies over the returned
/// content. An identical retry that re-fetched its attestation, or whose requested expiry differs
/// only before the max_expiry cap, must not get the old commitment glued to new content.
async fn rq_retry_content_matches_signature(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now().replace_nanosecond(0).unwrap();
    let w = wallet();
    let mut problems = Vec::new();

    let proofs = h.proofs(&[8, 4]);
    let blinds = h.blinds(&[8, 4]);
    let first = request(&proofs, &blinds, in_secs(t, 300), w);
    let (c1, s1) = h.svc.commit_to_swap(first.clone(), t).await.expect("commit A");
    core_signature::schnorr_verify_b64(&c1, &s1, &mint_xonly()).expect("first A verifies");
    let mut reattested = first.clone();
    reattested.inputs.attestation.coords_mac = [9u8; 32];
    reattested.inputs.attestation.signature = schnorr::Signature::from_slice(&[7u8; 64]).unwrap();
    if let Ok((c2, s2)) = h
        .svc
        .commit_to_swap(reattested, t + time::Duration::seconds(1))
        .await
    {
        if core_signature::schnorr_verify_b64(&c2, &s2, &mint_xonly()).is_err() {
            problems.push(format!(
                "re-attested retry: Ok, commitment does not verify over returned content \
                 (commitment == first: {}, content == first: {})",
                s2 == s1,
                c2 == c1
            ));
        }
    }

    let proofs = h.proofs(&[2]);
    let blinds = h.blinds(&[2]);
    let long = request(&proofs, &blinds, in_secs(t, 7200), w);
    let (c1, s1) = h.svc.commit_to_swap(long, t).await.expect("commit B");
    core_signature::schnorr_verify_b64(&c1, &s1, &mint_xonly()).expect("first B verifies");
    let capped = request(&proofs, &blinds, in_secs(t, 3600), w);
    if let Ok((c2, s2)) = h.svc.commit_to_swap(capped, t).await {
        if core_signature::schnorr_verify_b64(&c2, &s2, &mint_xonly()).is_err() {
            problems.push(format!(
                "expiry-cap retry: Ok, commitment does not verify over returned content \
                 (commitment == first: {})",
                s2 == s1
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}
#[tokio::test]
async fn rq_probe_retry_content_matches_signature() {
    per_backend!(rq_retry_content_matches_signature);
}

fn rounds(name: &str) -> usize {
    match name {
        "sqlx" => 4,
        _ => 25,
    }
}

/// Real parallelism: distinct commits over the same inputs, then every winner tries to swap.
/// At most one commit, one swap and one Clowder signal per input set.
async fn rq_concurrent_commits_mt(name: &'static str) {
    for round in 0..rounds(name) {
        let h = Arc::new(Harness::new(name).await);
        let t = now();
        let proofs = h.proofs(&[8, 2]);
        let mut tasks = Vec::new();
        for _ in 0..12 {
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
        let mut swaps = Vec::new();
        for (c, blinds) in wins.clone() {
            let (h, p) = (h.clone(), proofs.clone());
            swaps.push(tokio::spawn(async move { h.swap(&p, &blinds, c, t).await.is_ok() }));
        }
        let mut swapped = 0;
        for s in swaps {
            if s.await.unwrap() {
                swapped += 1;
            }
        }
        assert!(
            wins.len() <= 1 && swapped <= 1 && h.signals().len() <= 1,
            "round {round}: {} commits won over the same inputs, {swapped} swaps spent them, \
             {} Clowder signals",
            wins.len(),
            h.signals().len()
        );
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rq_probe_concurrent_commits_mt() {
    per_backend!(rq_concurrent_commits_mt);
}

/// Real parallelism: one commitment swapped by 8 concurrent requests. The swap is persisted
/// once, Clowder hears it once, the treasury receives the fee proofs once.
async fn rq_concurrent_swaps_mt(name: &'static str) {
    for round in 0..rounds(name) {
        let h = Arc::new(Harness::new(name).await);
        let t = now();
        let proofs = h.proofs(&[16]);
        let blinds = h.blinds(&[8, 4]);
        let (_, c) = h
            .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
            .await
            .expect("commit");
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let (h, p, b) = (h.clone(), proofs.clone(), blinds.clone());
            tasks.push(tokio::spawn(async move { h.swap(&p, &b, c, t).await }));
        }
        let mut oks = 0;
        for task in tasks {
            if task.await.unwrap().is_ok() {
                oks += 1;
            }
        }
        let finalized = h
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| *l == "swap_finalize:ok")
            .count();
        let fee_total: u64 = h
            .treasury
            .proofs
            .lock()
            .unwrap()
            .iter()
            .map(|p| u64::from(p.amount))
            .sum();
        assert!(
            finalized <= 1 && h.signals().len() <= 1 && fee_total <= 4,
            "round {round}: {oks}/8 swaps Ok, swap_finalize Ok {finalized}x, {} Clowder signals, \
             treasury fee total {fee_total} (expected 4)",
            h.signals().len()
        );
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rq_probe_concurrent_swaps_mt() {
    per_backend!(rq_concurrent_swaps_mt);
}

/// A commit that failed because Clowder did not sign must leave nothing behind: the same
/// wallet's identical retry a second later succeeds and swaps.
async fn rq_commit_retry_after_clowder_failure(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[8]);
    let blinds = h.blinds(&[8]);
    let w = wallet();
    let expiry = in_secs(t, 3000);
    h.clowder.fail_commits.store(1, Ordering::SeqCst);
    assert!(h.commit(&proofs, &blinds, expiry, w, t).await.is_err());
    let later = t + time::Duration::seconds(1);
    let state = h.svc.check_state(&proofs.ys().unwrap(), later).await.unwrap();
    let r = h.commit(&proofs, &blinds, expiry, w, later).await;
    let (_, c) = r.unwrap_or_else(|e| {
        panic!(
            "retry after Clowder refused to sign is rejected ({e}); inputs state {:?}",
            state.iter().map(|s| s.state).collect::<Vec<_>>()
        )
    });
    h.swap(&proofs, &blinds, c, later).await.expect("swap");
}
#[tokio::test]
async fn rq_probe_commit_retry_after_clowder_failure() {
    per_backend!(rq_commit_retry_after_clowder_failure);
}

/// Treasury store fails after the swap persisted: the request fails (good). Its retry must
/// not answer Ok while the failed store never happened.
async fn rq_treasury_failure_then_retry(name: &'static str) {
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
    let fee_total: u64 = h
        .treasury
        .proofs
        .lock()
        .unwrap()
        .iter()
        .map(|p| u64::from(p.amount))
        .sum();
    assert!(
        retry.is_err() || fee_total == 8,
        "retry Ok={} but treasury holds {fee_total} of 8 fee; signals {}",
        retry.is_ok(),
        h.signals().len()
    );
}
#[tokio::test]
async fn rq_probe_treasury_failure_then_retry() {
    per_backend!(rq_treasury_failure_then_retry);
}

/// Clowder must only be streamed swaps under the commitment that was persisted for them.
async fn rq_replay_with_forged_commitment(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[8]);
    let blinds = h.blinds(&[8]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    h.swap(&proofs, &blinds, c, t).await.expect("swap");
    let mut forged = Vec::new();
    for b in 1u8..=3 {
        let f = schnorr::Signature::from_slice(&[b; 64]).unwrap();
        if h.swap(&proofs, &blinds, f, t).await.is_ok() {
            forged.push(b);
        }
    }
    let commitments: Vec<_> = h.signals().iter().map(|s| s.commitment == c).collect();
    assert!(
        forged.is_empty() && commitments.iter().all(|x| *x),
        "swaps with never-issued commitments answered Ok: {forged:?}; Clowder signals \
         (true = real commitment): {commitments:?}"
    );
}
#[tokio::test]
async fn rq_probe_replay_with_forged_commitment() {
    per_backend!(rq_replay_with_forged_commitment);
}

/// A finished swap retried with its real commitment (past the nut19 cache) streams Clowder the
/// same spend again.
async fn rq_replay_after_success_resignals(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[16]);
    let blinds = h.blinds(&[8]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    h.swap(&proofs, &blinds, c, t).await.expect("swap");
    for _ in 0..3 {
        let _ = h.swap(&proofs, &blinds, c, t).await;
    }
    assert_eq!(
        h.signals().len(),
        1,
        "Clowder was streamed one finished swap {} times",
        h.signals().len()
    );
}
#[tokio::test]
async fn rq_probe_replay_after_success_resignals() {
    per_backend!(rq_replay_after_success_resignals);
}

/// Clowder unreachable after the swap persisted: the request fails, but the spend is stored
/// and the outputs are restorable (NUT-09). Nothing durable remembers Clowder was never told.
async fn rq_signal_failure_then_restore(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[8]);
    let blinds = h.blinds(&[8]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    h.clowder.fail_signals.store(1, Ordering::SeqCst);
    let r = h.swap(&proofs, &blinds, c, t).await;
    let restored = h.svc.search_signature(&blinds[0]).await.unwrap();
    let spent = h.spent(&proofs).await;
    assert!(
        !(spent[0] && h.signals().is_empty()),
        "swap returned {}, inputs spent={}, outputs restorable={}, Clowder signals={}",
        if r.is_ok() { "Ok" } else { "Err" },
        spent[0],
        restored.is_some(),
        h.signals().len()
    );
}
#[tokio::test]
async fn rq_probe_signal_failure_then_restore() {
    per_backend!(rq_signal_failure_then_restore);
}

/// Signed (Beta) commits follow the same rules: a failed store or a failed Clowder signature
/// fails the request.
async fn rq_signed_commit_failures(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let kp = core::generate_random_keypair();
    for mode in ["store", "clowder"] {
        let proofs = h.proofs(&[8]);
        let blinds = h.blinds(&[8]);
        let req = request(&proofs, &blinds, in_secs(t, 120), kp.public_key());
        let (payload, sig) = core_signature::serialize_n_schnorr_sign_borsh_msg(&req, &kp).unwrap();
        match mode {
            "store" => h.faults.commitment_store_errors.store(1, Ordering::SeqCst),
            _ => h.clowder.fail_commits.store(1, Ordering::SeqCst),
        }
        let r = h.svc.signed_commit_to_swap(payload, sig, t).await;
        assert!(r.is_err(), "signed commit Ok though {mode} failed");
    }
}
#[tokio::test]
async fn rq_probe_signed_commit_failures() {
    per_backend!(rq_signed_commit_failures);
}

/// Outside swaps: a keyset whose store fails must not have been announced to Clowder.
#[tokio::test]
async fn rq_probe_keyset_store_failure_streams_nothing() {
    let h = Harness::new("inmemory").await;
    h.faults.keys_store_errors.store(1, Ordering::SeqCst);
    let r = h.svc.create(cashu::CurrencyUnit::Sat, now(), None, 0).await;
    assert!(r.is_err(), "create Ok though keys_store failed");
    assert_eq!(
        h.clowder.new_keysets.load(Ordering::SeqCst),
        0,
        "Clowder was told of a keyset that was never stored"
    );
}
