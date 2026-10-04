//! Path / host tricks against the real core- and mint-service split, served by
//! `serve_split` on real TCP sockets.
mod adversarial_common;

use std::net::SocketAddr;

use adversarial_common::{free_addr, raw, statuses, wait_listening};
use axum::Router;
use bcr_common::client::admin::{
    core::admin_ep as core_admin, quote::admin_ep as quote_admin, treasury::admin_ep as treasury_admin,
};
use bcr_wdc_utils::serve::serve_split;

async fn start(web: Router, admin: Router) -> (SocketAddr, SocketAddr) {
    let (w, a) = (free_addr("127.0.0.1"), free_addr("127.0.0.1"));
    tokio::spawn(serve_split(web, admin, w, a, std::future::pending()));
    wait_listening(w).await;
    wait_listening(a).await;
    (w, a)
}

async fn core() -> (SocketAddr, SocketAddr) {
    let c = bcr_wdc_core_service::test_utils::test_controller();
    start(
        bcr_wdc_core_service::web_routes().with_state(c.clone()),
        bcr_wdc_core_service::admin_routes().with_state(c),
    )
    .await
}

async fn mint() -> (SocketAddr, SocketAddr) {
    let c = bcr_wdc_mint_service::test_utils::test_controller();
    start(
        bcr_wdc_mint_service::web_routes().with_state(c.clone()),
        bcr_wdc_mint_service::admin_routes().with_state(c),
    )
    .await
}

fn post(target: &str, host: &str, extra: &str) -> Vec<u8> {
    format!(
        "POST {target} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\n{extra}Content-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    )
    .into_bytes()
}

fn tricks(sign: &str, admin: SocketAddr) -> Vec<(String, Vec<u8>)> {
    let tail = sign.trim_start_matches("/admin");
    let enc = sign.replacen("/admin", "/%61dmin", 1);
    let slash_enc = sign.replace('/', "%2F");
    let mut v: Vec<(String, Vec<u8>)> = [
        format!("/{sign}"),
        format!("{sign}/"),
        sign.to_uppercase(),
        format!("/ADMIN{tail}"),
        enc,
        slash_enc,
        format!("/v1/..{sign}"),
        format!("/v1/%2e%2e{sign}"),
        format!("/.{sign}"),
        format!("{sign};x"),
        format!("{sign}?x=1"),
        format!("{sign}#x"),
        sign.replace('/', "\\"),
        format!("{sign}%00"),
        format!("http://{admin}{sign}"),
        format!("http://127.0.0.1{sign}"),
    ]
    .into_iter()
    .map(|t| (format!("POST {t}"), post(&t, "x", "")))
    .collect();
    v.push((format!("Host: {admin}"), post(sign, &admin.to_string(), "")));
    v.push((
        "X-Original-URL/X-Rewrite-URL/X-Forwarded-Prefix".into(),
        post(
            "/nope",
            "x",
            &format!("X-Original-URL: {sign}\r\nX-Rewrite-URL: {sign}\r\nX-Forwarded-Prefix: /admin\r\n"),
        ),
    ));
    v.push((
        "HTTP/1.0".into(),
        format!("POST {sign} HTTP/1.0\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}").into_bytes(),
    ));
    v.push((
        "pipelined GET /health + POST sign".into(),
        [
            b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n".to_vec(),
            post(sign, "x", ""),
        ]
        .concat(),
    ));
    v.push((
        "CL+TE smuggle".into(),
        format!(
            "POST /health HTTP/1.1\r\nHost: x\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\nPOST {sign} HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
        )
        .into_bytes(),
    ));
    v
}

/// Every response on the public port for an admin-path trick must be 404 or 400
/// (or no answer at all); the positive control proves the same request reaches the
/// handler on the admin port.
async fn assert_public_never_reaches(web: SocketAddr, admin: SocketAddr, sign: &str) {
    let (ctrl, _) = raw(admin, &post(sign, "x", "")).await;
    assert!(
        ctrl != 404 && ctrl != 0,
        "positive control: {sign} on the admin port answered {ctrl}"
    );
    let mut reached = vec![];
    for (name, bytes) in tricks(sign, admin) {
        let (_, text) = raw(web, &bytes).await;
        let all = statuses(&text);
        println!("web {name:?} -> {all:?}");
        let first_is_web_route = name.starts_with("pipelined") || name.starts_with("CL+TE");
        let bad: Vec<_> = all
            .iter()
            .enumerate()
            .filter(|(i, s)| !(**s == 404 || **s == 400 || (first_is_web_route && *i == 0)))
            .collect();
        if !bad.is_empty() {
            reached.push(format!("{name} -> {all:?}"));
        }
    }
    let h2 = reqwest::Client::builder()
        .http2_prior_knowledge()
        .build()
        .unwrap()
        .post(format!("http://{web}{sign}"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await;
    match h2 {
        Ok(r) => {
            println!("web h2 prior knowledge -> {}", r.status());
            if r.status().as_u16() != 404 {
                reached.push(format!("h2 -> {}", r.status()));
            }
        }
        Err(e) => println!("web h2 prior knowledge not served: {e}"),
    }
    assert!(reached.is_empty(), "public port answered non-404 for: {reached:#?}");
}

async fn assert_admin_serves_no_web(admin: SocketAddr, web_paths: &[&str]) {
    let mut served = vec![];
    for p in web_paths {
        let (s, _) = raw(
            admin,
            format!("GET {p} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await;
        println!("admin GET {p} -> {s}");
        if s != 404 {
            served.push(format!("{p} -> {s}"));
        }
    }
    assert!(served.is_empty(), "admin port serves web routes: {served:?}");
}

#[tokio::test]
async fn core_public_port_never_reaches_admin_sign() {
    let (w, a) = core().await;
    assert_public_never_reaches(w, a, core_admin::SIGN).await;
    assert_public_never_reaches(w, a, core_admin::BURN).await;
    assert_public_never_reaches(w, a, core_admin::NEW_KEYSET).await;
}

#[tokio::test]
async fn mint_public_port_never_reaches_admin_sign() {
    let (w, a) = mint().await;
    assert_public_never_reaches(w, a, core_admin::SIGN).await;
    assert_public_never_reaches(w, a, treasury_admin::FEES_STORE_PROOFS).await;
}

#[tokio::test]
async fn core_admin_port_serves_no_web_routes() {
    let (_, a) = core().await;
    assert_admin_serves_no_web(a, &["/health", "/v1/keysets", "/v1/keys/00ffffffffffffff", "/v1/checkstate"]).await;
}

#[tokio::test]
async fn mint_admin_port_serves_no_web_routes() {
    let (_, a) = mint().await;
    assert_admin_serves_no_web(a, &["/health", "/v1/keysets", "/v1/keys/00ffffffffffffff", "/v1/checkstate"]).await;
}

fn fill(path: &str) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let mut out = String::new();
    let mut skip = false;
    for c in path.chars() {
        match c {
            '{' => {
                skip = true;
                out.push_str(&id);
            }
            '}' => skip = false,
            _ if skip => {}
            _ => out.push(c),
        }
    }
    out
}

async fn quote() -> (SocketAddr, SocketAddr) {
    let c = bcr_wdc_quote_service::test_utils::test_controller();
    start(
        bcr_wdc_quote_service::web_routes::<bcr_wdc_quote_service::AppController>().with_state(c.clone()),
        bcr_wdc_quote_service::admin_routes::<bcr_wdc_quote_service::AppController>().with_state(c),
    )
    .await
}

async fn treasury() -> (SocketAddr, SocketAddr) {
    let c = bcr_wdc_treasury_service::test_utils::test_controller().await;
    start(
        bcr_wdc_treasury_service::web_routes::<bcr_wdc_treasury_service::AppController>().with_state(c.clone()),
        bcr_wdc_treasury_service::admin_routes::<bcr_wdc_treasury_service::AppController>().with_state(c),
    )
    .await
}

#[tokio::test]
async fn quote_public_port_never_reaches_admin() {
    let (w, a) = quote().await;
    for p in [quote_admin::ENABLE_MINTING, quote_admin::UPDATE, quote_admin::LIST] {
        assert_public_never_reaches(w, a, &fill(p)).await;
    }
}

#[tokio::test]
async fn treasury_public_port_never_reaches_admin() {
    let (w, a) = treasury().await;
    for p in [
        treasury_admin::TRY_HTLC_SWAP,
        treasury_admin::FEES_STORE_PROOFS,
        treasury_admin::REQUEST_TO_PAY_EBILL,
        treasury_admin::NEW_EBILL_MINTOP,
    ] {
        assert_public_never_reaches(w, a, &fill(p)).await;
    }
}
