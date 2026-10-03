//! Adversarial review probes for #1071 (mint-service): replay, races, partial failure.
//! Harness copied from probes/mint_swap_probes.rs. Each probe asserts the correct behaviour.
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

/// Holds the next `hold` callers until `open` releases them.
struct Gate {
    hold: AtomicUsize,
    held: AtomicUsize,
    release: tokio::sync::Semaphore,
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            hold: AtomicUsize::new(0),
            held: AtomicUsize::new(0),
            release: tokio::sync::Semaphore::new(0),
        }
    }
}

impl Gate {
    async fn pass(&self) {
        if take(&self.hold) {
            self.held.fetch_add(1, Ordering::SeqCst);
            self.release.acquire().await.unwrap().forget();
        }
    }
    async fn wait_held(&self, n: usize) {
        while self.held.load(Ordering::SeqCst) < n {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }
    fn open(&self, n: usize) {
        self.release.add_permits(n);
    }
}

#[derive(Default)]
struct ClowderState {
    signals: Mutex<Vec<Signal>>,
    commits: AtomicUsize,
    violations: Mutex<Vec<String>>,
    fail_signals: AtomicUsize,
    commit_gate: Gate,
    signal_gate: Gate,
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
        self.state.commit_gate.pass().await;
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
        self.state.signal_gate.pass().await;
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
    service_with(repo, observed, clowder, log, TStamp::UNIX_EPOCH)
}

fn service_with(
    repo: Arc<dyn Repository>,
    observed: Arc<dyn Repository>,
    clowder: Arc<ClowderState>,
    log: Arc<Mutex<Vec<String>>>,
    settle_window_deadline: TStamp,
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
        settle_window_deadline,
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


fn forged_amounts(proofs: &[cashu::Proof]) -> Vec<cashu::Proof> {
    proofs
        .iter()
        .cloned()
        .map(|mut p| {
            p.amount = cashu::Amount::from(1u64);
            p
        })
        .collect()
}

fn foreign_signals(h: &Harness, issued: &[schnorr::Signature]) -> Vec<schnorr::Signature> {
    h.signals()
        .iter()
        .map(|s| s.commitment)
        .filter(|c| !issued.contains(c))
        .collect()
}

// ------------------------------------------------------------------ (a) replay of finalized swaps

/// A finished swap WITH fees: lowering the replayed proofs' amounts (nothing checks them on the
/// recovery path) removes the fee outputs, so a never-issued commitment is accepted anyway.
async fn adv_replay_fee_swap_forged_amount(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[16]);
    let blinds = h.blinds(&[8, 4]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    h.swap(&proofs, &blinds, c, t).await.expect("swap");
    let forged_c = schnorr::Signature::from_slice(&[0x5a; 64]).unwrap();
    let plain = h.swap(&proofs, &blinds, forged_c, t).await;
    let forged = h.swap(&forged_amounts(&proofs), &blinds, forged_c, t).await;
    let foreign = foreign_signals(&h, &[c]);
    assert!(
        forged.is_err() && foreign.is_empty(),
        "fee-bearing swap replayed with never-issued commitment: real amounts -> {}, amounts forged to 1 -> {}; \
         Clowder signals under foreign commitments: {}",
        if plain.is_ok() { "Ok" } else { "Err" },
        if forged.is_ok() { "Ok" } else { "Err" },
        foreign.len()
    );
}
#[tokio::test]
async fn probe_adv_replay_fee_swap_forged_amount() {
    per_backend!(adv_replay_fee_swap_forged_amount);
}

/// Each replay with a fresh fake commitment produces another Clowder swap event: unbounded.
async fn adv_replay_unbounded_signals(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[16]);
    let blinds = h.blinds(&[8, 4]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    h.swap(&proofs, &blinds, c, t).await.expect("swap");
    let forged = forged_amounts(&proofs);
    let mut oks = 0;
    for _ in 0..25 {
        let fake = signatures_test::random_schnorr_signature();
        if h.swap(&forged, &blinds, fake, t).await.is_ok() {
            oks += 1;
        }
    }
    assert_eq!(
        h.signals().len(),
        1,
        "25 replays with random commitments: {oks} returned Ok, Clowder got {} swap events for one swap",
        h.signals().len()
    );
}
#[tokio::test]
async fn probe_adv_replay_unbounded_signals() {
    per_backend!(adv_replay_unbounded_signals);
}

/// The genuine commitment of ANOTHER finished swap is accepted for these inputs.
async fn adv_replay_other_swaps_commitment(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let p1 = h.proofs(&[8]);
    let b1 = h.blinds(&[8]);
    let (_, c1) = h.commit(&p1, &b1, in_secs(t, 120), wallet(), t).await.expect("c1");
    h.swap(&p1, &b1, c1, t).await.expect("s1");
    let p2 = h.proofs(&[4]);
    let b2 = h.blinds(&[4]);
    let (_, c2) = h.commit(&p2, &b2, in_secs(t, 120), wallet(), t).await.expect("c2");
    h.swap(&p2, &b2, c2, t).await.expect("s2");
    let r = h.swap(&p1, &b1, c2, t).await;
    let crossed = h
        .signals()
        .iter()
        .filter(|s| s.commitment == c2 && s.ys == p1.ys().unwrap())
        .count();
    assert!(
        r.is_err() && crossed == 0,
        "swap 1's inputs replayed under swap 2's commitment: {} and {crossed} Clowder signals",
        if r.is_ok() { "Ok" } else { "Err" }
    );
}
#[tokio::test]
async fn probe_adv_replay_other_swaps_commitment() {
    per_backend!(adv_replay_other_swaps_commitment);
}

/// A swap that never happened: inputs of two different swaps, a subset of their outputs in a
/// different order, a made-up commitment. Clowder is streamed it as a real swap.
async fn adv_fabricated_swap_mixing(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let p1 = h.proofs(&[8, 2]);
    let b1 = h.blinds(&[8, 2]);
    let (_, c1) = h.commit(&p1, &b1, in_secs(t, 120), wallet(), t).await.expect("c1");
    h.swap(&p1, &b1, c1, t).await.expect("s1");
    let p2 = h.proofs(&[4]);
    let b2 = h.blinds(&[4]);
    let (_, c2) = h.commit(&p2, &b2, in_secs(t, 120), wallet(), t).await.expect("c2");
    h.swap(&p2, &b2, c2, t).await.expect("s2");
    let mut inputs = forged_amounts(&p1);
    inputs.extend(forged_amounts(&p2));
    let outputs = vec![b2[0].clone(), b1[1].clone()];
    let fake = signatures_test::random_schnorr_signature();
    let r = h.swap(&inputs, &outputs, fake, t).await;
    let foreign = foreign_signals(&h, &[c1, c2]);
    assert!(
        r.is_err() && foreign.is_empty(),
        "fabricated swap (inputs of 2 swaps, 2 of their 3 outputs, fake commitment) -> {}; Clowder signals: {}",
        if r.is_ok() { "Ok" } else { "Err" },
        foreign.len()
    );
}
#[tokio::test]
async fn probe_adv_fabricated_swap_mixing() {
    per_backend!(adv_fabricated_swap_mixing);
}

/// During the settle window an Unsigned swap is refused; the recovery path skips that check.
async fn adv_replay_in_settle_window(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[8]);
    let blinds = h.blinds(&[8]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    h.swap(&proofs, &blinds, c, t).await.expect("swap");
    let windowed = service_with(
        h.repo.clone(),
        h.repo.clone(),
        h.clowder.clone(),
        h.log.clone(),
        t + time::Duration::days(1),
    );
    let fake = signatures_test::random_schnorr_signature();
    let r = windowed
        .swap(h.treasury.as_ref(), proofs.clone(), blinds.clone(), fake, t)
        .await;
    assert!(
        matches!(r, Err(Error::ServiceUnavailable)) && foreign_signals(&h, &[c]).is_empty(),
        "unsigned replay inside the settle window: {r:?}; foreign Clowder signals {}",
        foreign_signals(&h, &[c]).len()
    );
}
#[tokio::test]
async fn probe_adv_replay_in_settle_window() {
    per_backend!(adv_replay_in_settle_window);
}

// ------------------------------------------------------------------ (b) concurrent finalize

/// Repeated rounds of 24 concurrent swaps of one commitment: swap_finalize must succeed once.
async fn adv_concurrent_swaps_many(name: &'static str) {
    let mut bad = Vec::new();
    for round in 0..8 {
        let h = Arc::new(Harness::new(name).await);
        let t = now();
        let proofs = h.proofs(&[8, 4]);
        let blinds = h.blinds(&[8, 4]);
        let (_, c) = h
            .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
            .await
            .expect("commit");
        let mut tasks = Vec::new();
        for _ in 0..24 {
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
        if finalized > 1 {
            bad.push(format!(
                "round {round}: finalize ok x{finalized}, {oks} Ok, {} signals, treasury fee proofs {}",
                h.signals().len(),
                h.treasury.proofs.lock().unwrap().len()
            ));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("; "));
}
#[tokio::test]
async fn probe_adv_concurrent_swaps_many() {
    adv_concurrent_swaps_many("surreal").await;
}

// ------------------------------------------------------------------ (c) races the branch added

/// Concurrent retries after a lost Clowder signal: each one re-signals Clowder.
async fn adv_concurrent_recovery_retries(name: &'static str) {
    let h = Arc::new(Harness::new(name).await);
    let t = now();
    let proofs = h.proofs(&[16]);
    let blinds = h.blinds(&[8, 4]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    h.clowder.fail_signals.store(1, Ordering::SeqCst);
    assert!(h.swap(&proofs, &blinds, c, t).await.is_err());
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
    assert_eq!(
        h.signals().len(),
        1,
        "8 concurrent retries after one lost signal: {oks} Ok, {} Clowder swap events",
        h.signals().len()
    );
}
#[tokio::test]
async fn probe_adv_concurrent_recovery_retries() {
    per_backend!(adv_concurrent_recovery_retries);
}

/// A swap still in flight (persisted, Clowder call pending) and its own retry: Clowder hears
/// the swap twice.
async fn adv_inflight_swap_and_retry(name: &'static str) {
    let h = Arc::new(Harness::new(name).await);
    let t = now();
    let proofs = h.proofs(&[8]);
    let blinds = h.blinds(&[8]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
        .await
        .expect("commit");
    h.clowder.signal_gate.hold.store(1, Ordering::SeqCst);
    let first = {
        let (h, p, b) = (h.clone(), proofs.clone(), blinds.clone());
        tokio::spawn(async move { h.swap(&p, &b, c, t).await })
    };
    h.clowder.signal_gate.wait_held(1).await;
    let retry = h.swap(&proofs, &blinds, c, t).await;
    h.clowder.signal_gate.open(1);
    let first = first.await.unwrap();
    assert_eq!(
        h.signals().len(),
        1,
        "in-flight swap {} + its retry {}: {} Clowder swap events",
        if first.is_ok() { "Ok" } else { "Err" },
        if retry.is_ok() { "Ok" } else { "Err" },
        h.signals().len()
    );
}
#[tokio::test]
async fn probe_adv_inflight_swap_and_retry() {
    per_backend!(adv_inflight_swap_and_retry);
}

/// A commit whose Clowder call outlasts its own expiry: its reservation is reaped, an admin
/// reserves the inputs, then the late commitment_store lands. The admin reservation must hold.
async fn adv_late_commit_vs_admin_reservation(name: &'static str) {
    let h = Arc::new(Harness::new(name).await);
    let t = now();
    let proofs = h.proofs(&[8]);
    let ys = proofs.ys().unwrap();
    h.clowder.commit_gate.hold.store(1, Ordering::SeqCst);
    let late = {
        let h = h.clone();
        let (p, b) = (proofs.clone(), h.blinds(&[8]));
        tokio::spawn(async move { h.commit(&p, &b, in_secs(t, 2), wallet(), t).await })
    };
    h.clowder.commit_gate.wait_held(1).await;
    let t2 = t + time::Duration::seconds(10);
    h.svc.check_state(&ys, t2).await.expect("check_state reaps");
    h.svc
        .reserve(ys.clone(), t2 + time::Duration::seconds(600))
        .await
        .expect("admin reserve after the late commit's reservation expired");
    h.clowder.commit_gate.open(1);
    let late = late.await.unwrap();
    let t3 = t2 + time::Duration::seconds(1);
    h.svc.check_state(&ys, t3).await.expect("check_state");
    let still_reserved = h.repo.ys_contains(&ys).await.unwrap();
    let other = h
        .commit(&proofs, &h.blinds(&[8]), in_secs(t3, 120), wallet(), t3)
        .await;
    assert!(
        still_reserved == vec![true] && other.is_err(),
        "late commit returned {}; admin reservation still held: {still_reserved:?}; \
         a new commit over the admin-reserved input: {}",
        if late.is_ok() { "Ok" } else { "Err" },
        if other.is_ok() { "Ok" } else { "Err" }
    );
}
#[tokio::test]
async fn probe_adv_late_commit_vs_admin_reservation() {
    per_backend!(adv_late_commit_vs_admin_reservation);
}

/// A commitment that exists without a ys reservation (stored before this deploy, or by any
/// path other than commit_to_swap): a different commit over its inputs reaches Clowder, which
/// signs a second commitment over inputs already committed.
async fn adv_unreserved_commitment_reaches_clowder(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[8]);
    let ys = proofs.ys().unwrap();
    let old_out = h.blinds(&[8]);
    let old = signatures_test::random_schnorr_signature();
    let fps: Vec<wire_keys::ProofFingerprint> = proofs
        .iter()
        .cloned()
        .map(wire_keys::ProofFingerprint::try_from)
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    h.repo
        .commitment_store(
            ys,
            old_out.iter().map(|b| b.blinded_secret).collect(),
            t + time::Duration::seconds(600),
            wallet().into(),
            old,
            bcr_common::wire::attestation::fp_digest(&fps),
            SignatureOwner::Unsigned,
        )
        .await
        .expect("pre-existing commitment");
    let r = h
        .commit(&proofs, &h.blinds(&[8]), in_secs(t, 120), wallet(), t)
        .await;
    let commits = h.clowder.commits.load(Ordering::SeqCst);
    assert!(
        r.is_err() && commits == 0,
        "commit over inputs of an existing commitment: {}; Clowder asked to sign {commits} time(s)",
        if r.is_ok() { "Ok" } else { "Err" }
    );
}
#[tokio::test]
async fn probe_adv_unreserved_commitment_reaches_clowder() {
    per_backend!(adv_unreserved_commitment_reaches_clowder);
}

// ------------------------------------------------------------------ (d) partial failure / restart

/// Clowder down when the swap persisted, and the wallet never retries (it got a 5xx): after a
/// restart nothing ever tells Clowder the inputs were spent.
async fn adv_lost_signal_no_retry(name: &'static str) {
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
    let restarted = h.restarted();
    let _ = restarted.check_state(&proofs.ys().unwrap(), t).await;
    assert!(
        !h.spent(&proofs).await.contains(&true) || !h.signals().is_empty(),
        "swap returned {} and its inputs are spent, but Clowder was never told (no outbox, no retry)",
        if r.is_ok() { "Ok" } else { "Err" }
    );
}
#[tokio::test]
async fn probe_adv_lost_signal_no_retry() {
    per_backend!(adv_lost_signal_no_retry);
}

// ------------------------------------------------------------------ (e) hostile input

/// Duplicate ys and empty inputs in a commit request are refused before anything is reserved.
async fn adv_hostile_commit_inputs(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let p = h.proofs(&[8]);
    let dup = vec![p[0].clone(), p[0].clone()];
    let r = h.commit(&dup, &h.blinds(&[8, 8]), in_secs(t, 120), wallet(), t).await;
    assert!(r.is_err(), "duplicate ys accepted");
    let r = h.commit(&[], &h.blinds(&[8]), in_secs(t, 120), wallet(), t).await;
    assert!(r.is_err(), "empty inputs accepted");
    assert_eq!(h.clowder.commits.load(Ordering::SeqCst), 0);
    assert_eq!(h.repo.ys_contains(&p.ys().unwrap()).await.unwrap(), vec![false]);
    let r = h.swap(&dup, &h.blinds(&[16]), signatures_test::random_schnorr_signature(), t).await;
    assert!(r.is_err(), "swap with duplicate proofs accepted");
}
#[tokio::test]
async fn probe_adv_hostile_commit_inputs() {
    per_backend!(adv_hostile_commit_inputs);
}

// ------------------------------------------------------------------ over HTTP

/// Over HTTP: a fee-bearing swap's proofs (amounts lowered) and outputs, POSTed with 10 random
/// commitments, are answered 200 each time. The NUT-19 cache keys on the commitment, so it
/// never absorbs these.
#[tokio::test]
async fn probe_adv_http_forged_replays() {
    let (info, keyset) = core_tests::generate_random_ecash_keyset();
    let (_server, ctrl) =
        test_utils::build_test_server(Some(keys_utils::to_entry(info, keyset.clone()))).await;
    let url = _server.server_address().expect("address");
    let client = CoreClient::new(url);
    let c_proofs = core_tests::generate_random_ecash_proofs(&keyset, &[cashu::Amount::from(16u64)]);
    let fps: Vec<wire_keys::ProofFingerprint> = c_proofs
        .iter()
        .cloned()
        .map(wire_keys::ProofFingerprint::try_from)
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    let amounts: Vec<_> = [8u64, 4].iter().map(|a| cashu::Amount::from(*a)).collect();
    let blinds: Vec<ecash::BlindedMessage> = signatures_test::generate_blinds(keyset.id.into(), &amounts)
        .into_iter()
        .map(|g| ecash::BlindedMessage::from(g.0))
        .collect();
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
    client.swap(proofs, blinds.clone(), c).await.expect("swap");
    let forged: Vec<_> = forged_amounts(&c_proofs).into_iter().map(ecash::Proof::from).collect();
    let mut oks = 0;
    for _ in 0..10 {
        let fake = signatures_test::random_schnorr_signature();
        if client.swap(forged.clone(), blinds.clone(), fake).await.is_ok() {
            oks += 1;
        }
    }
    let _ = ctrl;
    assert_eq!(oks, 0, "{oks}/10 POST swap with never-issued commitments answered 200");
}

/// Storage level, no service: 24 concurrent surreal swap_finalize calls for one commitment.
/// swap_finalize is unchanged since a15061b, so this also characterises the base.
#[tokio::test]
async fn probe_adv_surreal_swap_finalize_race() {
    let mut bad = Vec::new();
    for round in 0..8 {
        let repo = surreal_repo().await;
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &[cashu::Amount::from(8u64)]);
        let blinds: Vec<cashu::BlindedMessage> =
            signatures_test::generate_blinds(keyset.id.into(), &[cashu::Amount::from(8u64)])
                .into_iter()
                .map(|g| g.0)
                .collect();
        let c = signatures_test::random_schnorr_signature();
        repo.commitment_store(
            proofs.ys().unwrap(),
            blinds.iter().map(|b| b.blinded_secret).collect(),
            now() + time::Duration::seconds(600),
            wallet().into(),
            c,
            [0u8; 32],
            SignatureOwner::Unsigned,
        )
        .await
        .expect("commitment");
        let sig = cashu::BlindSignature {
            amount: cashu::Amount::from(8u64),
            keyset_id: keyset.id.into(),
            c: blinds[0].blinded_secret,
            dleq: None,
        };
        let mut tasks = Vec::new();
        for _ in 0..24 {
            let (repo, p) = (repo.clone(), proofs.clone());
            let stored = vec![StoredSignature {
                y: blinds[0].blinded_secret,
                signature: sig.clone(),
            }];
            tasks.push(tokio::spawn(async move { repo.swap_finalize(p, stored, c).await }));
        }
        let mut oks = 0;
        for task in tasks {
            if task.await.unwrap().is_ok() {
                oks += 1;
            }
        }
        if oks > 1 {
            bad.push(format!("round {round}: {oks} of 24 swap_finalize Ok"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("; "));
}
