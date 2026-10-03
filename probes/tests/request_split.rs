//! Request lens: every admin route is served only on the admin listener, every web
//! route only on the web listener, and no route of the base's combined router was
//! dropped by the split. Routes are read from the source (branch and base a15061b),
//! so a route added or removed is probed without editing this file.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use bcr_common::client::{core, quote, treasury};
#[allow(unused_imports)]
use bcr_common::client::{core as core_ep, treasury as cl_treasury, treasury as treasury_ep};
use reqwest::Method;

const BASE: &str = "a15061b";
const KID: &str = "009a1f293253e41e";
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

macro_rules! dict {
    ($($e:expr),* $(,)?) => { vec![$((stringify!($e), $e)),*] };
}

/// Every path expression any service lib.rs uses, mapped to its value.
fn dictionary() -> Vec<(&'static str, &'static str)> {
    let mut d = dict![
        "/health",
        core::web_ep::KEYSET_INFO_V1,
        core::web_ep::LIST_KEYSET_INFO_V1,
        core::web_ep::KEYS_V1,
        core::web_ep::KEYS_V2,
        core::web_ep::RESTORE_V1,
        core::web_ep::SWAP_V1,
        core::web_ep::SWAP_COMMIT_V1,
        core::web_ep::SIGNED_SWAP_COMMIT_V1,
        core::web_ep::CHECK_STATE_V1,
        core::admin_ep::NEW_KEYSET,
        core::admin_ep::SIGN,
        core::admin_ep::VERIFY_PROOF,
        core::admin_ep::VERIFY_FINGERPRINT,
        core::admin_ep::BURN,
        core::admin_ep::RECOVER,
        core::admin_ep::RESERVE,
        core_ep::web_ep::KEYSET_INFO_V1,
        core_ep::web_ep::LIST_KEYSET_INFO_V1,
        core_ep::web_ep::KEYS_V1,
        core_ep::web_ep::KEYS_V2,
        core_ep::web_ep::RESTORE_V1,
        core_ep::web_ep::SWAP_V1,
        core_ep::web_ep::SWAP_COMMIT_V1,
        core_ep::web_ep::SIGNED_SWAP_COMMIT_V1,
        core_ep::web_ep::CHECK_STATE_V1,
        core_ep::admin_ep::NEW_KEYSET,
        core_ep::admin_ep::SIGN,
        core_ep::admin_ep::VERIFY_PROOF,
        core_ep::admin_ep::VERIFY_FINGERPRINT,
        core_ep::admin_ep::BURN,
        core_ep::admin_ep::RECOVER,
        core_ep::admin_ep::RESERVE,
        treasury_ep::admin_ep::FEES_STORE_PROOFS,
        treasury_ep::admin_ep::FEES_TOKEN,
        quote::web_ep::ENQUIRE_V1,
        quote::web_ep::LOOKUP_V1,
        quote::web_ep::RESOLVE_V1,
        quote::admin_ep::LIST,
        quote::admin_ep::LOOKUP,
        quote::admin_ep::UPDATE,
        quote::admin_ep::ENABLE_MINTING,
        quote::admin_ep::SHARED_EBILL_HISTORY,
        cl_treasury::web_ep::EXCHANGE_ONLINE_V1,
        cl_treasury::web_ep::EXCHANGE_OFFLINE_V1,
        cl_treasury::web_ep::EXCHANGE_OFFLINE_REDEEM_V1,
        cl_treasury::web_ep::MELTQUOTE_ONCHAIN_V1,
        cl_treasury::web_ep::MELT_ONCHAIN_V1,
        cl_treasury::web_ep::MELT_ONCHAIN_ESTIMATE_V1,
        cl_treasury::web_ep::MELT_ONCHAIN_CONFIG_V1,
        cl_treasury::web_ep::MINTQUOTE_ONCHAIN_V1,
        cl_treasury::web_ep::MINT_ONCHAIN_V1,
        cl_treasury::web_ep::EBILLMINT_V1,
    ];
    d.extend(dict![
        cl_treasury::admin_ep::REQUEST_TO_PAY_EBILL,
        cl_treasury::admin_ep::TRY_HTLC_SWAP,
        cl_treasury::admin_ep::NEW_EBILL_MINTOP,
        cl_treasury::admin_ep::LIST_EBILL_MINTOPS,
        cl_treasury::admin_ep::EBILL_MINTOP_STATUS,
        cl_treasury::admin_ep::FEES_STORE_PROOFS,
        cl_treasury::admin_ep::FEES_TOKEN,
        cl_treasury::admin_ep::DENIED_MELTOPS,
        cl_treasury::admin_ep::DENIED_MELTOP,
        cl_treasury::admin_ep::FOREIGN_BALANCE,
    ]);
    d
}

/// Every constant of bcr-common's core, quote and treasury `admin_ep` modules (a299a7a).
fn all_admin_ep() -> Vec<&'static str> {
    vec![
        core::admin_ep::NEW_KEYSET,
        core::admin_ep::SIGN,
        core::admin_ep::VERIFY_PROOF,
        core::admin_ep::VERIFY_FINGERPRINT,
        core::admin_ep::RECOVER,
        core::admin_ep::BURN,
        core::admin_ep::RESERVE,
        quote::admin_ep::LIST,
        quote::admin_ep::LOOKUP,
        quote::admin_ep::UPDATE,
        quote::admin_ep::ENABLE_MINTING,
        quote::admin_ep::SHARED_EBILL_HISTORY,
        treasury::admin_ep::EBILL_MINTOP_STATUS,
        treasury::admin_ep::LIST_EBILL_MINTOPS,
        treasury::admin_ep::NEW_EBILL_MINTOP,
        treasury::admin_ep::REQUEST_TO_PAY_EBILL,
        treasury::admin_ep::TRY_HTLC_SWAP,
        treasury::admin_ep::FEES_STORE_PROOFS,
        treasury::admin_ep::FEES_TOKEN,
        treasury::admin_ep::DENIED_MELTOPS,
        treasury::admin_ep::DENIED_MELTOP,
        treasury::admin_ep::FOREIGN_BALANCE,
    ]
}

fn resolve(expr: &str) -> &'static str {
    dictionary()
        .into_iter()
        .find(|(e, _)| *e == expr)
        .unwrap_or_else(|| panic!("probe dictionary lacks route expression `{expr}`"))
        .1
}

/// `(path expression, method)` of every `.route(` in `src`.
fn parse_routes(src: &str) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    let mut rest = src;
    while let Some(i) = rest.find(".route(") {
        rest = &rest[i + ".route(".len()..];
        let comma = rest.find(',').expect("comma");
        let expr = rest[..comma].trim().to_string();
        let after = rest[comma + 1..].trim_start();
        let paren = after.find('(').expect("paren");
        let method = after[..paren].rsplit("::").next().unwrap().trim().to_uppercase();
        out.insert((expr, method));
    }
    out
}

fn fn_body<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src
        .find(&format!("pub fn {name}"))
        .unwrap_or_else(|| panic!("no fn {name}"));
    let body = &src[start..];
    let end = body.find("\n}\n").expect("fn end");
    &body[..end]
}

fn repo() -> String {
    format!("{}/..", env!("CARGO_MANIFEST_DIR"))
}

fn branch_src(krate: &str) -> String {
    std::fs::read_to_string(format!("{}/crates/{krate}/src/lib.rs", repo())).expect("read lib.rs")
}

fn base_src(krate: &str) -> String {
    let out = std::process::Command::new("git")
        .args(["-C", &repo(), "show", &format!("{BASE}:crates/{krate}/src/lib.rs")])
        .output()
        .expect("git show");
    assert!(out.status.success(), "git show failed");
    String::from_utf8(out.stdout).unwrap()
}

fn fill(path: &str) -> String {
    let mut p = path.replace("{kid}", KID);
    while let (Some(a), Some(b)) = (p.find('{'), p.find('}')) {
        p.replace_range(a..=b, &uuid::Uuid::new_v4().to_string());
    }
    p
}

fn free_addr() -> SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

struct Split {
    web: SocketAddr,
    admin: SocketAddr,
    _tx: tokio::sync::oneshot::Sender<()>,
}

async fn spawn(web: Router, admin: Router) -> Split {
    let (web_addr, admin_addr) = (free_addr(), free_addr());
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        bcr_wdc_utils::serve::serve_split(web, admin, web_addr, admin_addr, async {
            let _ = rx.await;
        })
        .await
        .expect("serve_split");
    });
    for addr in [web_addr, admin_addr] {
        for _ in 0..100 {
            if tokio::net::TcpStream::connect(addr).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    Split {
        web: web_addr,
        admin: admin_addr,
        _tx: tx,
    }
}

#[derive(Debug)]
struct Outcome {
    status: Option<u16>,
    empty_body: bool,
}

impl Outcome {
    /// The router matched the path (handler ran, or 405 for another method).
    fn routed(&self) -> bool {
        !(self.status == Some(404) && self.empty_body)
    }
    /// The router matched path and method: a handler ran (or is still running).
    fn served(&self) -> bool {
        self.routed() && self.status != Some(405)
    }
    fn show(&self) -> String {
        match self.status {
            Some(s) if s == 404 && self.empty_body => "404(unrouted)".into(),
            Some(s) => format!("{s}"),
            None => "timeout(handler running)".into(),
        }
    }
}

async fn hit(addr: SocketAddr, method: &str, path: &str) -> Outcome {
    let m = Method::from_bytes(method.as_bytes()).unwrap();
    let mut req = reqwest::Client::new()
        .request(m.clone(), format!("http://{addr}{}", fill(path)))
        .timeout(Duration::from_secs(5));
    if m != Method::GET && m != Method::DELETE {
        req = req.header("content-type", "application/json").body("{}");
    }
    match req.send().await {
        Ok(r) => {
            let status = r.status().as_u16();
            let body = r.bytes().await.unwrap_or_default();
            Outcome {
                status: Some(status),
                empty_body: body.is_empty(),
            }
        }
        Err(e) if e.is_timeout() => Outcome {
            status: None,
            empty_body: false,
        },
        Err(e) => panic!("{method} {path} on {addr}: {e}"),
    }
}

/// Runs every case for one service and returns the failures, printing every result.
async fn check_service(name: &str, krate: &str, s: &Split) -> Vec<String> {
    let src = branch_src(krate);
    let web: BTreeSet<_> = parse_routes(fn_body(&src, "web_routes"));
    let admin: BTreeSet<_> = parse_routes(fn_body(&src, "admin_routes"));
    let mut fails = Vec::new();
    let mut log = |line: String, ok: bool| {
        println!("[{name}] {} {line}", if ok { "ok  " } else { "FAIL" });
        if !ok {
            fails.push(format!("[{name}] {line}"));
        }
    };

    let web_paths: BTreeSet<&str> = web.iter().map(|(e, _)| resolve(e)).collect();
    let mut admin_paths: BTreeSet<&str> = admin.iter().map(|(e, _)| resolve(e)).collect();
    admin_paths.extend(all_admin_ep());

    for path in &admin_paths {
        for m in METHODS {
            let o = hit(s.web, m, path).await;
            let also_web = if web_paths.contains(path) { " [also a web path]" } else { "" };
            log(format!("web   {m:6} {path} -> {} (want 404){also_web}", o.show()), !o.routed());
        }
    }
    for (expr, m) in &admin {
        let path = resolve(expr);
        let o = hit(s.admin, m, path).await;
        log(format!("admin {m:6} {path} -> {} (want served)", o.show()), o.served());
    }
    for (expr, m) in &web {
        let path = resolve(expr);
        let o = hit(s.web, m, path).await;
        log(format!("web   {m:6} {path} -> {} (want served)", o.show()), o.served());
        if admin_paths.contains(path) {
            continue;
        }
        for m2 in METHODS {
            let o = hit(s.admin, m2, path).await;
            log(format!("admin {m2:6} {path} -> {} (want 404)", o.show()), !o.routed());
        }
    }
    let h = hit(s.web, "GET", "/health").await;
    let base_has_health = base_src(krate).contains("\"/health\"");
    log(
        format!(
            "web   GET    /health -> {} (want 200; base had /health: {base_has_health})",
            h.show()
        ),
        h.status == Some(200) || !base_has_health,
    );
    fails
}

/// Every route of the base's merged router is served by exactly one listener, and the
/// branch's web + admin routes equal the base's route set.
async fn check_relation(name: &str, krate: &str, s: &Split) -> Vec<String> {
    let base = parse_routes(&base_src(krate));
    let src = branch_src(krate);
    let mut branch = parse_routes(fn_body(&src, "web_routes"));
    branch.extend(parse_routes(fn_body(&src, "admin_routes")));
    let mut fails = Vec::new();
    let canon = |set: &BTreeSet<(String, String)>| -> BTreeSet<(String, String)> {
        set.iter()
            .map(|(e, m)| (resolve(e).to_string(), m.clone()))
            .collect()
    };
    let (base_c, branch_c) = (canon(&base), canon(&branch));
    for missing in base_c.difference(&branch_c) {
        fails.push(format!("[{name}] base route {missing:?} missing from web_routes+admin_routes"));
    }
    for extra in branch_c.difference(&base_c) {
        fails.push(format!("[{name}] branch route {extra:?} not in base router"));
    }
    println!("[{name}] base routes: {}", base_c.len());
    for (path, m) in &base_c {
        let w = hit(s.web, m, path).await;
        let a = hit(s.admin, m, path).await;
        let n = [w.served(), a.served()].iter().filter(|b| **b).count();
        let line = format!(
            "base {m:6} {path}: web={} admin={} (want exactly one served)",
            w.show(),
            a.show()
        );
        println!("[{name}] {} {line}", if n == 1 { "ok  " } else { "FAIL" });
        if n != 1 {
            fails.push(format!("[{name}] {line}"));
        }
    }
    fails
}

async fn core_split() -> Split {
    let c = bcr_wdc_core_service::test_utils::test_controller();
    spawn(
        bcr_wdc_core_service::web_routes().with_state(c.clone()),
        bcr_wdc_core_service::admin_routes().with_state(c),
    )
    .await
}

async fn mint_split() -> Split {
    let c = bcr_wdc_mint_service::test_utils::test_controller();
    spawn(
        bcr_wdc_mint_service::web_routes().with_state(c.clone()),
        bcr_wdc_mint_service::admin_routes().with_state(c),
    )
    .await
}

async fn quote_split() -> Split {
    let c = bcr_wdc_quote_service::test_utils::test_controller();
    spawn(
        bcr_wdc_quote_service::web_routes().with_state(c.clone()),
        bcr_wdc_quote_service::admin_routes().with_state(c),
    )
    .await
}

async fn treasury_split() -> Split {
    let c = bcr_wdc_treasury_service::test_utils::test_controller().await;
    spawn(
        bcr_wdc_treasury_service::web_routes().with_state(c.clone()),
        bcr_wdc_treasury_service::admin_routes().with_state(c),
    )
    .await
}

fn verdict(fails: Vec<String>) {
    assert!(fails.is_empty(), "{} failure(s):\n{}", fails.len(), fails.join("\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn core_admin_only_on_admin_listener() {
    let s = core_split().await;
    verdict(check_service("core", "bcr-wdc-core-service", &s).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn mint_admin_only_on_admin_listener() {
    let s = mint_split().await;
    verdict(check_service("mint", "bcr-wdc-mint-service", &s).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn quote_admin_only_on_admin_listener() {
    let s = quote_split().await;
    verdict(check_service("quote", "bcr-wdc-quote-service", &s).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn treasury_admin_only_on_admin_listener() {
    let s = treasury_split().await;
    verdict(check_service("treasury", "bcr-wdc-treasury-service", &s).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn core_split_keeps_every_base_route() {
    let s = core_split().await;
    verdict(check_relation("core", "bcr-wdc-core-service", &s).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn mint_split_keeps_every_base_route() {
    let s = mint_split().await;
    verdict(check_relation("mint", "bcr-wdc-mint-service", &s).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn quote_split_keeps_every_base_route() {
    let s = quote_split().await;
    verdict(check_relation("quote", "bcr-wdc-quote-service", &s).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn treasury_split_keeps_every_base_route() {
    let s = treasury_split().await;
    verdict(check_relation("treasury", "bcr-wdc-treasury-service", &s).await);
}
