//! INTEGRATION lens: every outbound call an internal caller makes must land on a
//! listener that serves it, now that core/mint/quote/treasury serve admin routes on
//! a separate socket. Each caller's client struct is wired exactly as its init code
//! wires it (web URL for the public client, admin URL for the admin client).

mod integration_common;

use std::str::FromStr;
use std::sync::{atomic::AtomicU64, Arc};

use bcr_common::{
    cashu,
    client::{core::Client as CoreClient, ebill::Client as EbillClient, treasury::Client as TreasuryClient},
    core::test_utils as core_tests,
    ecash, time,
    wire::keys as wire_keys,
    TStamp,
};
use integration_common::*;

struct Inputs {
    kid: cashu::Id,
    blinds: Vec<cashu::BlindedMessage>,
    proofs: Vec<cashu::Proof>,
    ys: Vec<cashu::PublicKey>,
    fps: Vec<wire_keys::ProofFingerprint>,
}

fn inputs() -> Inputs {
    let (kinfo, keyset) = core_tests::generate_random_ecash_keyset();
    let kid: cashu::Id = kinfo.id.into();
    let amounts = [cashu::Amount::from(8u64)];
    let blinds = bcr_wdc_utils::signatures::test_utils::generate_blinds(kid, &amounts)
        .into_iter()
        .map(|b| b.0)
        .collect();
    let proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
    let ys = proofs.iter().map(|p| p.y().expect("y")).collect();
    let fps = proofs
        .iter()
        .cloned()
        .map(wire_keys::ProofFingerprint::try_from)
        .collect::<Result<_, _>>()
        .expect("fps");
    Inputs {
        kid,
        blinds,
        proofs,
        ys,
        fps,
    }
}

fn assert_all_served(log: &Log, what: &str) {
    let hits = log.hits();
    println!("{what}:\n{}", log.dump());
    assert!(!hits.is_empty(), "{what}: no request reached any listener");
    let unserved = log.unserved();
    assert!(
        unserved.is_empty(),
        "{what}: {} call(s) landed on a listener with no route for them:\n{}",
        unserved.len(),
        unserved
            .iter()
            .map(|h| format!("  [{}] {} {} -> {}", h.listener, h.method, h.path, h.status))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[tokio::test]
async fn sanity_tap_marks_unrouted_requests() {
    let log = Log::default();
    let (core, _) = core_split(&log).await;
    let s = stub("stub", &log).await;
    let cl = reqwest::Client::new();
    cl.post(core.web.join("/admin/keys/sign").unwrap()).send().await.unwrap();
    cl.get(core.web.join("/health").unwrap()).send().await.unwrap();
    cl.get(s.url.join("/anything").unwrap()).send().await.unwrap();
    let hits = log.hits();
    println!("{}", log.dump());
    assert_eq!(hits.len(), 3);
    assert!(hits[0].unserved && !hits[1].unserved && hits[2].unserved);
}

async fn drive_treasury_core_clients(core: &Split, ebill_stub: &Single) {
    use bcr_wdc_treasury_service::{ebill, foreign, onchain, vault};
    let core_cl = Arc::new(CoreClient::new(core.web.clone()));
    let core_admin_cl = Arc::new(CoreClient::new(core.admin.clone()));
    let i = inputs();

    let oc = onchain::WildcatCl {
        core_cl: core_cl.clone(),
        core_admin_cl: core_admin_cl.clone(),
    };
    use onchain::WildcatClient as _;
    let _ = oc.verify_fingerprints(&i.fps).await;
    let _ = oc.verify_proofs(&i.proofs).await;
    let _ = oc.check_spendable(i.ys.clone()).await;
    let _ = oc.sign(i.blinds.clone()).await;
    let _ = oc.burn(i.proofs.clone()).await;
    let _ = oc.recover(i.proofs.clone()).await;
    let _ = oc
        .reserve_inputs(i.ys.clone(), TStamp::now_utc() + time::Duration::minutes(5))
        .await;
    let _ = oc.keyset_info(i.kid).await;
    let _ = oc.keyset(i.kid).await;
    let _ = oc.get_active_keyset().await;

    let eb = ebill::WildcatCl {
        core: core_cl.clone(),
        core_admin: core_admin_cl.clone(),
        ebill: Box::new(EbillClient::new(ebill_stub.url.clone())),
    };
    {
        use ebill::WildcatClient as _;
        let _ = eb.info(i.kid).await;
        let _ = eb.sign(&i.blinds).await;
        let _ = eb.burn(i.proofs.clone()).await;
        let _ = eb.recover(i.proofs.clone()).await;
    }

    let fc = foreign::clients::CoreCl {
        core: core_cl.clone(),
        core_admin: core_admin_cl.clone(),
    };
    {
        use foreign::KeysClient as _;
        let date = time::OffsetDateTime::now_utc().date() + time::Duration::days(30);
        let _ = fc.get_keyset_with_expiration(date).await;
        let _ = fc.sign(&i.blinds).await;
        let _ = fc.burn(i.proofs.clone()).await;
        let _ = fc.proof_states(i.ys.clone()).await;
    }

    let vc = vault::WildcatCl {
        core: core_cl.clone(),
    };
    {
        use vault::WildcatClient as _;
        let _ = vc.check_spent(i.ys.clone()).await;
    }
}

/// treasury -> core-service split (core_url = web, core_admin_url = admin).
#[tokio::test]
async fn treasury_clients_reach_core_split() {
    let log = Log::default();
    let (core, _) = core_split(&log).await;
    let ebill_log = Log::default();
    let ebill_stub = stub("ebill", &ebill_log).await;
    drive_treasury_core_clients(&core, &ebill_stub).await;
    assert_all_served(&log, "treasury -> core split");
}

/// treasury -> mint-service split (mint-service serves the same core routes).
#[tokio::test]
async fn treasury_clients_reach_mint_split() {
    let log = Log::default();
    let mint = mint_split(&log).await;
    let ebill_log = Log::default();
    let ebill_stub = stub("ebill", &ebill_log).await;
    drive_treasury_core_clients(&mint, &ebill_stub).await;
    assert_all_served(&log, "treasury -> mint split");
}

/// quote-service's WildcatCl is private: drive the same bcr-common calls on clients
/// wired as quote's init_app wires them (core_url, core_admin_url, treasury_admin_url).
#[tokio::test]
async fn quote_clients_reach_core_and_treasury_split() {
    let log = Log::default();
    let (core, _) = core_split(&log).await;
    let treasury = treasury_split(&log).await;
    let core_public = CoreClient::new(core.web.clone());
    let core_admin = CoreClient::new(core.admin.clone());
    let treasury_cl = TreasuryClient::new(treasury.admin.clone());
    let i = inputs();
    let date = time::OffsetDateTime::now_utc().date() + time::Duration::days(30);
    let _ = core_public
        .list_keyset_info(wire_keys::KeysetInfoFilters::default())
        .await;
    let _ = core_admin.new_keyset(Some(date), 0).await;
    let _ = core_public.keys(i.kid).await;
    let blinds: Vec<ecash::BlindedMessage> = i.blinds.iter().cloned().map(From::from).collect();
    let _ = core_admin.sign(&blinds).await;
    let qid = uuid::Uuid::new_v4();
    let pk = bcr_common::core::generate_random_keypair().public_key();
    let _ = treasury_cl
        .new_ebill_mint_operation(
            qid,
            i.kid,
            cashu::PublicKey::from(pk),
            cashu::Amount::from(8u64),
            core_tests::random_bill_id(),
        )
        .await;
    let _ = treasury_cl.ebill_mint_operation_status(qid).await;
    let _ = treasury_cl.fees_store_proofs(i.proofs.clone()).await;
    assert_all_served(&log, "quote -> core/treasury split");
}

/// core-service -> treasury: core's own `clients::TreasuryCl` (fees storage on every
/// swap) still has a single `treasury_url`, unchanged by the branch. Pointed at
/// treasury's public listener (what `treasury_url` named at base).
#[tokio::test]
async fn core_treasury_client_reaches_treasury_via_treasury_url() {
    use bcr_wdc_core_service::clients::{TreasuryCl, TreasuryService as _};
    let log = Log::default();
    let treasury = treasury_split(&log).await;
    let tcl = TreasuryCl {
        cl: Box::new(TreasuryClient::new(treasury.web.clone())),
    };
    let res = tcl.store_proofs(vec![]).await;
    println!("store_proofs via treasury_url(public): {res:?}");
    assert_all_served(&log, "core -> treasury (treasury_url = treasury public listener)");
}

/// Same call against the base topology (one merged treasury router): served.
#[tokio::test]
async fn core_treasury_client_was_served_at_base() {
    use bcr_wdc_core_service::clients::{TreasuryCl, TreasuryService as _};
    let log = Log::default();
    let treasury = treasury_merged(&log).await;
    let tcl = TreasuryCl {
        cl: Box::new(TreasuryClient::new(treasury.url.clone())),
    };
    let res = tcl.store_proofs(vec![]).await;
    println!("store_proofs via base merged router: {res:?}");
    assert_all_served(&log, "core -> treasury (base merged router)");
}

/// End to end in core-service: an unsigned swap with core's treasury client pointed at
/// treasury's public listener. Swap finalises (inputs spent) before
/// `treasury.store_proofs` fails, so the wallet gets an error and its inputs are gone.
#[tokio::test]
async fn core_swap_with_treasury_url_on_public_listener() {
    use bcr_wdc_core_service::{clients, service, test_utils};
    let log = Log::default();
    let treasury = treasury_split(&log).await;
    let base = test_utils::test_controller();
    let svc = service::Service {
        repository: base.service.repository.clone(),
        clowder: Box::new(clients::DummyClowderClient),
        treasury: Box::new(clients::TreasuryCl {
            cl: Box::new(TreasuryClient::new(treasury.web.clone())),
        }),
        keygen: base.service.keygen.clone(),
        min_keyset_fees_ppk: AtomicU64::new(0),
        max_expiry: time::Duration::seconds(3600),
        alpha_id: test_utils::mint_kp().public_key(),
        settle_window_deadline: TStamp::UNIX_EPOCH,
    };
    let (kinfo, keyset) = core_tests::generate_random_ecash_keyset();
    let entry = bcr_wdc_utils::MintKeysEntry {
        id: kinfo.id.into(),
        unit: kinfo.unit.clone(),
        active: kinfo.active,
        valid_from: kinfo.valid_from,
        derivation_path: kinfo.derivation_path.clone(),
        derivation_path_index: kinfo.derivation_path_index,
        amounts: kinfo.amounts.clone(),
        input_fee_ppk: kinfo.input_fee_ppk,
        final_expiry: kinfo.final_expiry,
        keys: keyset.keys.clone(),
    };
    svc.repository.keys_store(entry).await.expect("store");
    let amounts = vec![cashu::Amount::from(8_u64)];
    let blinds: Vec<cashu::BlindedMessage> =
        bcr_wdc_utils::signatures::test_utils::generate_blinds(kinfo.id.into(), &amounts)
            .into_iter()
            .map(|b| b.0)
            .collect();
    let proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
    let fps: Vec<wire_keys::ProofFingerprint> = proofs
        .iter()
        .cloned()
        .map(wire_keys::ProofFingerprint::try_from)
        .collect::<Result<_, _>>()
        .unwrap();
    let now = time::OffsetDateTime::now_utc();
    let request = bcr_common::wire::swap::SwapCommitmentRequest {
        inputs: test_utils::attested_fingerprints(fps),
        outputs: blinds.iter().cloned().map(From::from).collect(),
        expiry: (now + time::Duration::minutes(2)).unix_timestamp() as u64,
        wallet_key: bcr_common::core::generate_random_keypair().public_key(),
    };
    let (_, commitment) = svc.commit_to_swap(request, now).await.expect("commit");
    let ys: Vec<cashu::PublicKey> = proofs.iter().map(|p| p.y().unwrap()).collect();
    let res = svc.swap(proofs.clone(), blinds.clone(), commitment, now).await;
    let states = svc.check_state(&ys, now).await.expect("check_state");
    println!("swap result: {res:?}\ninput states after swap: {states:?}");
    println!("{}", log.dump());
    assert!(
        res.is_ok(),
        "swap failed with treasury_url on treasury's public listener: {res:?}; \
         input states afterwards: {:?}",
        states.iter().map(|s| s.state).collect::<Vec<_>>()
    );
}

/// wallet-aggregator (public) calls `treasury_client.try_htlc` (admin_ep::TRY_HTLC_SWAP)
/// for HTLC inputs of a swap, through `treasury_client_url`, unchanged by the branch.
#[tokio::test]
async fn wallet_aggregator_try_htlc_reaches_treasury_public() {
    let log = Log::default();
    let treasury = treasury_split(&log).await;
    let tcl = TreasuryClient::new(treasury.web.clone());
    let res = tcl.try_htlc(String::from("00")).await;
    println!("try_htlc via treasury public: {res:?}");
    assert_all_served(&log, "wallet-aggregator -> treasury (treasury_client_url = public)");
}

#[tokio::test]
async fn wallet_aggregator_try_htlc_was_served_at_base() {
    let log = Log::default();
    let treasury = treasury_merged(&log).await;
    let tcl = TreasuryClient::new(treasury.url.clone());
    let res = tcl.try_htlc(String::from("00")).await;
    println!("try_htlc via base merged: {res:?}");
    assert_all_served(&log, "wallet-aggregator -> treasury (base merged)");
}

/// admin-aggregator: real AppController::new (pre-flight included) against real
/// core/quote/treasury splits, every endpoint driven over TCP.
#[tokio::test]
async fn admin_aggregator_every_endpoint_reaches_a_serving_listener() {
    use bcr_wdc_admin_aggregator::{endpoints as ep, routes, AppConfig, AppController};
    let log = Log::default();
    let (core, _) = core_split(&log).await;
    let quote = quote_split(&log).await;
    let treasury = treasury_split(&log).await;
    let side = Log::default();
    let ebill = stub("ebill", &side).await;
    let clowder = stub("clowder", &side).await;
    let cfg = AppConfig {
        core_url: core.web.clone(),
        core_admin_url: core.admin.clone(),
        quotes_admin_url: quote.admin.clone(),
        ebill_url: ebill.url.clone(),
        clowder_url: clowder.url.clone(),
        treasury_admin_url: treasury.admin.clone(),
    };
    let ctrl = AppController::new(cfg).await;
    let agg = spawn_single(routes(ctrl)).await;

    let kid = cashu::Id::from_str("009a1f293253e41e").unwrap();
    let qid = uuid::Uuid::new_v4();
    let bid = core_tests::random_bill_id();
    let pk = bcr_common::core::generate_random_keypair().public_key();
    let sub = |p: &str| {
        p.replace("{kid}", &kid.to_string())
            .replace("{qid}", &qid.to_string())
            .replace("{bid}", &bid.to_string())
            .replace("{pk}", &pk.to_string())
            .replace("{rid}", &qid.to_string())
            .replace("{fname}", "f.pdf")
    };
    let reqtopay = serde_json::json!({
        "ebill_id": bid.to_string(), "amount": 1000, "deadline": "2030-01-01T00:00:00Z"
    });
    let calls: Vec<(&str, &str, Option<serde_json::Value>)> = vec![
        ("GET", ep::MINT_INFO, None),
        ("GET", ep::KEYSET_INFO, None),
        ("GET", ep::LIST_KEYSET_INFOS, None),
        ("GET", ep::GET_CREDIT_QUOTE, None),
        ("GET", ep::LIST_CREDIT_QUOTES, None),
        ("PUT", ep::UPDATE_CREDIT_QUOTE, Some(serde_json::json!({"action": "Deny"}))),
        ("PATCH", ep::ENABLE_QUOTE_MINTING, None),
        ("GET", ep::GET_SHARED_EBILL_HISTORY, None),
        ("GET", ep::GET_IDENTITY, None),
        ("GET", ep::GET_EBILL, None),
        ("GET", ep::LIST_EBILLS, None),
        ("GET", ep::GET_EBILL_BALANCE, None),
        ("GET", ep::GET_EBILL_ENDORSEMENTS, None),
        ("GET", ep::GET_EBILL_PAYMENTSTATUS, None),
        ("GET", ep::GET_EBILL_PAYMENTACTIONS, None),
        ("GET", ep::GET_EBILL_HISTORY, None),
        ("GET", ep::GET_CLOWDER_INFO, None),
        ("GET", ep::GET_CLOWDER_ALPHAS, None),
        ("GET", ep::GET_CLOWDER_BETAS, None),
        ("GET", ep::GET_CLOWDER_MYSTATUS, None),
        ("GET", ep::MINT_OP_STATUS, None),
        ("GET", ep::LIST_MINT_OPS, None),
        ("POST", ep::POST_EBILL_REQTOPAY, Some(reqtopay)),
        ("GET", ep::DENIED_MELTOPS, None),
        ("DELETE", ep::DENIED_MELTOP, None),
        ("GET", ep::FEES_TOKEN, None),
        ("GET", ep::FOREIGN_BALANCE, None),
    ];
    let http = reqwest::Client::new();
    let mut no_backend = vec![];
    println!("pre-flight:\n{}", log.dump());
    assert!(log.unserved().is_empty(), "pre-flight hit an unserved route:\n{}", log.dump());
    for (method, path, body) in calls {
        let before = log.hits().len() + side.hits().len();
        let u = agg.url.join(&sub(path)).unwrap();
        let rb = http.request(reqwest::Method::from_str(method).unwrap(), u);
        let rb = match body {
            Some(b) => rb.json(&b),
            None => rb,
        };
        let resp = rb.send().await.unwrap();
        let after = log.hits().len() + side.hits().len();
        println!("aggregator {method} {path} -> {}", resp.status());
        if after == before {
            no_backend.push(format!("{method} {path} -> {}", resp.status()));
        }
    }
    println!("ebill/clowder stub hits:\n{}", side.dump());
    assert!(no_backend.is_empty(), "aggregator endpoints that reached no backend: {no_backend:#?}");
    assert_all_served(&log, "admin-aggregator -> core/quote/treasury split");
}
