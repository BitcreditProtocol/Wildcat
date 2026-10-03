//! Probes for ticket #1071 against bcr-wdc-core-service: persist before sign/stream, and a
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
use bcr_wdc_core_service::{
    clients::{ClowderClient, DummyClowderClient, PublicKeyOwner, TreasuryService},
    factory::Factory as KeysFactory,
    service::Service,
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

struct SharedTreasury(Arc<RecTreasury>);

#[async_trait]
impl TreasuryService for SharedTreasury {
    async fn store_proofs(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        self.0.store_proofs(proofs).await
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
        let treasury: Arc<RecTreasury> = Arc::default();
        let svc = Arc::new(service(
            repo.clone(),
            inner.clone(),
            clowder.clone(),
            log.clone(),
            treasury.clone(),
        ));
        Self {
            svc,
            repo,
            faults,
            clowder,
            log,
            treasury,
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
            self.treasury.clone(),
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
    treasury: Arc<RecTreasury>,
) -> Service {
    Service {
        treasury: Box::new(SharedTreasury(treasury)),
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


// ------------------------------------------------------------------ review probes (integration lens)

fn surreal_cfg() -> bcr_wdc_utils::surreal::DBConnConfig {
    bcr_wdc_utils::surreal::DBConnConfig {
        connection: String::from("mem://"),
        namespace: String::from("probe"),
        database: String::from("probe"),
    }
}

/// The surreal -> sqlx migration (bin/migrate.rs) replays `commitments` with
/// `commitment_store`, then `reserved_ys` with `ys_store`, aborting the whole migration
/// (exit 1) on the first error. A swap commitment pending in SurrealDB at migration time
/// must migrate.
#[tokio::test]
async fn probe_int_migration_with_pending_swap_commitment() {
    let surreal = Arc::new(
        persistence::surreal::Repository::new(surreal_cfg())
            .await
            .expect("surreal"),
    );
    let h = Harness::over(surreal.clone()).await;
    let t = now();
    let proofs = h.proofs(&[8, 2]);
    let blinds = h.blinds(&[8, 2]);
    let (_, c) = h
        .commit(&proofs, &blinds, in_secs(t, 600), wallet(), t)
        .await
        .expect("commit");

    let commitments = surreal.dump_commitments().await.expect("dump commitments");
    let reserved = surreal.dump_reserved_ys().await.expect("dump reserved ys");
    println!(
        "surreal holds {} commitments and {} reserved ys after one swap commit",
        commitments.len(),
        reserved.len()
    );

    let pg = sqlx_repo().await;
    for commitment in commitments {
        pg.commitment_store(
            commitment.inputs,
            commitment.outputs,
            commitment.expiration,
            commitment.wallet_key,
            commitment.signature,
            commitment.fp_digest,
            commitment.signed,
        )
        .await
        .expect("migrate commitment");
    }
    let mut failures = Vec::new();
    for (y, deadline) in reserved {
        if let Err(error) = pg.ys_store(vec![y], deadline).await {
            failures.push(format!("Failed to migrate reserved y {y}: {error}"));
        }
    }
    assert!(
        failures.is_empty(),
        "migrate.rs would exit(1) here:\n{}",
        failures.join("\n")
    );
    pg.commitment_load(&c).await.expect("migrated commitment loads");
}

/// A Clowder whose commit call runs a hook first, standing in for whatever else touches the
/// repository while Clowder is signing (another request's check_state, a treasury reserve).
struct HookClowder {
    repo: Arc<dyn Repository>,
    hook: Mutex<Option<(TStamp, TStamp)>>,
    hook_log: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ClowderClient for HookClowder {
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
        let hook = self.hook.lock().unwrap().take();
        if let Some((clean_at, foreign_deadline)) = hook {
            let ys: Vec<cashu::PublicKey> = request.inputs.inputs.iter().map(|fp| fp.y).collect();
            self.repo.commitment_clean_expired(clean_at).await.unwrap();
            self.repo.ys_clean_expired(clean_at).await.unwrap();
            let r = self.repo.ys_store(ys, foreign_deadline).await;
            self.hook_log
                .lock()
                .unwrap()
                .push(format!("foreign reserve while Clowder signs: {r:?}"));
        }
        DummyClowderClient.commit_to_swap(request).await
    }
    async fn signal_swap_event(
        &self,
        _inputs: Vec<cashu::Proof>,
        _outputs: Vec<cashu::BlindedMessage>,
        _fees: Vec<cashu::BlindSignature>,
        _commitment: schnorr::Signature,
        _signatures: Vec<cashu::BlindSignature>,
    ) -> Result<()> {
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

/// A wallet asks for a 1s expiry; Clowder takes longer than that. Meanwhile another request's
/// check_state reaps the swap's now-expired reservation and the treasury reserves the same
/// inputs for a melt (`reserve_inputs`, deadline 10 min). That melt reservation must survive
/// the late commitment_store: the inputs stay Reserved and no other swap can commit them.
async fn foreign_reservation_survives_late_commit(name: &'static str) {
    let repo = backend(name).await;
    let (mut kinfo, mut keyset) = core_tests::generate_random_ecash_keyset();
    kinfo.input_fee_ppk = 0;
    keyset.input_fee_ppk = 0;
    repo.keys_store(keys_utils::to_entry(kinfo, keyset.clone()))
        .await
        .unwrap();
    let t = now();
    let hook_log = Arc::new(Mutex::new(Vec::new()));
    let svc = Service {
        treasury: Box::new(RecTreasury::default()),
        repository: repo.clone(),
        clowder: Box::new(HookClowder {
            repo: repo.clone(),
            hook: Mutex::new(Some((
                t + time::Duration::seconds(5),
                t + time::Duration::seconds(600),
            ))),
            hook_log: hook_log.clone(),
        }),
        keygen: KeysFactory::new(&[7u8; 32], bitcoin::bip32::DerivationPath::default()),
        min_keyset_fees_ppk: AtomicU64::new(0),
        max_expiry: time::Duration::hours(1),
        alpha_id: test_utils::mint_kp().public_key(),
        settle_window_deadline: TStamp::UNIX_EPOCH,
    };
    let amounts = [cashu::Amount::from(8u64)];
    let proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
    let ys = proofs.ys().unwrap();
    let blinds = |_: ()| -> Vec<cashu::BlindedMessage> {
        signatures_test::generate_blinds(keyset.id.into(), &amounts)
            .into_iter()
            .map(|g| g.0)
            .collect()
    };
    let late = svc
        .commit_to_swap(request(&proofs, &blinds(()), in_secs(t, 1), wallet()), t)
        .await;
    println!("[{name}] {:?}", hook_log.lock().unwrap());
    println!("[{name}] late commit: {:?}", late.as_ref().map(|r| r.1));
    let reserved_now = repo.ys_contains(&ys).await.unwrap();
    let later = t + time::Duration::seconds(5);
    let states: Vec<_> = svc
        .check_state(&ys, later)
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.state)
        .collect();
    let other = svc
        .commit_to_swap(request(&proofs, &blinds(()), in_secs(later, 120), wallet()), later)
        .await;
    assert!(
        reserved_now == vec![true]
            && states == vec![cashu::State::Reserved]
            && other.is_err(),
        "treasury's melt reservation was lost: ys_contains right after the late commit = \
         {reserved_now:?}; check_state 5s later = {states:?}; a new swap commit over the melt's \
         inputs = {:?}",
        other.map(|r| r.1)
    );
}
#[tokio::test]
async fn probe_int_foreign_reservation_survives_late_commit() {
    per_backend!(foreign_reservation_survives_late_commit);
}

/// Inputs reserved by an admin/treasury `reserve`: base rejected the commit with
/// InvalidInput (400, "One or more proofs are not unspent"). Records what the branch returns.
async fn reserved_input_error_kind(name: &'static str) {
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
    println!("[{name}] commit over reserved inputs -> {r:?}");
    assert!(
        matches!(r, Err(Error::InvalidInput(_))),
        "commit over reserved inputs no longer returns InvalidInput (HTTP 400): {r:?}"
    );
}
#[tokio::test]
async fn probe_int_reserved_input_error_kind() {
    per_backend!(reserved_input_error_kind);
}

/// After a swap commitment is stored (sqlx claims the swap's own reservation: deadline NULL),
/// the commitment's expiry alone must free the inputs on every backend.
async fn expiry_frees_claimed_inputs(name: &'static str) {
    let h = Harness::new(name).await;
    let t = now();
    let proofs = h.proofs(&[8]);
    let ys = proofs.ys().unwrap();
    let (_, old) = h
        .commit(&proofs, &h.blinds(&[8]), in_secs(t, 60), wallet(), t)
        .await
        .expect("commit");
    let later = t + time::Duration::seconds(180);
    let states: Vec<_> = h
        .svc
        .check_state(&ys, later)
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.state)
        .collect();
    assert_eq!(states, vec![cashu::State::Unspent], "inputs still held after expiry");
    let blinds = h.blinds(&[8]);
    let (_, new) = h
        .commit(&proofs, &blinds, in_secs(later, 60), wallet(), later)
        .await
        .expect("commit after expiry");
    assert!(h.swap(&proofs, &h.blinds(&[8]), old, later).await.is_err());
    h.swap(&proofs, &blinds, new, later).await.expect("swap");
}
#[tokio::test]
async fn probe_int_expiry_frees_claimed_inputs() {
    per_backend!(expiry_frees_claimed_inputs);
}

/// Fee outputs are now derived from HMAC(master, commitment) with counter 0 per keyset. A swap
/// paying fees in two keysets, and two swaps in a row, must not collide on fee blinded
/// messages (core_signatures is keyed by blinded secret).
async fn fee_outputs_do_not_collide(name: &'static str) {
    let h = Harness::new(name).await;
    let (mut kinfo2, mut keyset2) = core_tests::generate_random_ecash_keyset();
    kinfo2.input_fee_ppk = 0;
    keyset2.input_fee_ppk = 0;
    h.repo
        .keys_store(keys_utils::to_entry(kinfo2, keyset2.clone()))
        .await
        .unwrap();
    let t = now();
    for _ in 0..2 {
        let mut proofs = h.proofs(&[16]);
        proofs.extend(core_tests::generate_random_ecash_proofs(
            &keyset2,
            &[cashu::Amount::from(16u64)],
        ));
        let mut blinds = h.blinds(&[8]);
        blinds.extend(
            signatures_test::generate_blinds(keyset2.id.into(), &[cashu::Amount::from(8u64)])
                .into_iter()
                .map(|g| g.0),
        );
        let (_, c) = h
            .commit(&proofs, &blinds, in_secs(t, 120), wallet(), t)
            .await
            .expect("commit");
        h.swap(&proofs, &blinds, c, t).await.expect("two-keyset swap with fees");
    }
    let fee_total: u64 = h
        .treasury
        .proofs
        .lock()
        .unwrap()
        .iter()
        .map(|p| u64::from(p.amount))
        .sum();
    assert_eq!(fee_total, 32);
}
#[tokio::test]
async fn probe_int_fee_outputs_do_not_collide() {
    per_backend!(fee_outputs_do_not_collide);
}
