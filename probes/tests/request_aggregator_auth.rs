//! Request lens: admin-aggregator binds one public listener carrying admin operations,
//! so every route it serves (but /health) must refuse a request with no credentials
//! and one with a wrong bearer token (401). Routes are read from its src/lib.rs, so a
//! route added later is probed too. The aggregator is served exactly as main.rs does
//! (`axum::serve` of `routes(app)`), against stub core/quote/treasury backends.

use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use axum::{
    routing::{get, patch, post},
    Json, Router,
};
use bcr_common::{
    cashu,
    client::{core as cl_core, quote as cl_quote, treasury as cl_treasury, Url},
    ecash,
    wire::{keys as wire_keys, quotes as wire_quotes, treasury as wire_treasury},
};
use bcr_wdc_admin_aggregator::{endpoints, routes, AppConfig, AppController};

const KID: &str = "009a1f293253e41e";

fn keyset_info() -> ecash::KeySetInfo {
    ecash::KeySetInfo {
        id: cashu::Id::from_str(KID).unwrap().into(),
        unit: cl_core::Client::currency_unit(),
        active: true,
        input_fee_ppk: 1,
        final_expiry: None,
    }
}

async fn serve(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    addr
}

fn url(addr: SocketAddr) -> Url {
    Url::parse(&format!("http://{addr}")).unwrap()
}

async fn aggregator() -> SocketAddr {
    let core_web = serve(
        Router::new()
            .route(
                cl_core::web_ep::LIST_KEYSET_INFO_V1,
                get(|| async {
                    Json(wire_keys::KeysetInfoListResponse {
                        keysets: vec![keyset_info()],
                    })
                }),
            )
            .route(
                cl_core::web_ep::KEYSET_INFO_V1,
                get(|| async { Json(keyset_info()) }),
            ),
    )
    .await;
    let core_admin = serve(Router::new().route(
        cl_core::admin_ep::NEW_KEYSET,
        post(|| async { Json(keyset_info()) }),
    ))
    .await;
    let quote_admin = serve(Router::new().route(
        cl_quote::admin_ep::ENABLE_MINTING,
        patch(|| async { Json(wire_quotes::EnableMintingResponse {}) }),
    ))
    .await;
    let treasury_admin = serve(Router::new().route(
        cl_treasury::admin_ep::FOREIGN_BALANCE,
        get(|| async { Json(wire_treasury::ForeignBalanceResponse { balances: vec![] }) }),
    ))
    .await;
    let unused = serve(Router::new()).await;
    let ctrl = AppController::new(AppConfig {
        core_url: url(core_web),
        core_admin_url: url(core_admin),
        quotes_admin_url: url(quote_admin),
        ebill_url: url(unused),
        clowder_url: url(unused),
        treasury_admin_url: url(treasury_admin),
    })
    .await;
    serve(routes(ctrl)).await
}

fn lib_src() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/bcr-wdc-admin-aggregator/src/lib.rs"
    ))
    .unwrap()
}

/// `NAME -> "/path"` for every `pub const` in `pub mod endpoints`.
fn endpoint_consts(src: &str) -> Vec<(String, String)> {
    let start = src.find("pub mod endpoints").expect("endpoints module");
    let body = &src[start..start + src[start..].find("\n}\n").unwrap()];
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find("pub const ") {
        rest = &rest[i + "pub const ".len()..];
        let name = rest[..rest.find(':').unwrap()].trim().to_string();
        let q1 = rest.find('"').unwrap();
        let q2 = q1 + 1 + rest[q1 + 1..].find('"').unwrap();
        out.push((name, rest[q1 + 1..q2].to_string()));
        rest = &rest[q2..];
    }
    out
}

/// `(NAME, METHOD)` of every `.route(endpoints::NAME, method(..))` in `routes`.
fn routed(src: &str) -> Vec<(String, String)> {
    let start = src.find("pub fn routes").expect("routes fn");
    let body = &src[start..start + src[start..].find("\n}\n").unwrap()];
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find(".route(") {
        rest = &rest[i + ".route(".len()..];
        let comma = rest.find(',').unwrap();
        let expr = rest[..comma].trim();
        let name = expr.strip_prefix("endpoints::").unwrap_or(expr).to_string();
        let after = rest[comma + 1..].trim_start();
        let method = after[..after.find('(').unwrap()].trim().to_uppercase();
        out.push((name, method));
    }
    out
}

fn fill(path: &str) -> String {
    let mut p = path.replace("{kid}", KID);
    while let (Some(a), Some(b)) = (p.find('{'), p.find('}')) {
        p.replace_range(a..=b, &uuid::Uuid::new_v4().to_string());
    }
    p
}

async fn status(addr: SocketAddr, method: &str, path: &str, auth: Option<&str>) -> String {
    let m = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
    let mut req = reqwest::Client::new()
        .request(m.clone(), format!("http://{addr}{}", fill(path)))
        .timeout(Duration::from_secs(10));
    if let Some(a) = auth {
        req = req.header("authorization", a);
    }
    if m != reqwest::Method::GET && m != reqwest::Method::DELETE {
        req = req.header("content-type", "application/json").body("{}");
    }
    match req.send().await {
        Ok(r) => r.status().as_u16().to_string(),
        Err(e) if e.is_timeout() => "timeout".into(),
        Err(e) => format!("error({e})"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_aggregator_route_refuses_missing_or_wrong_credentials() {
    let src = lib_src();
    let consts = endpoint_consts(&src);
    assert_eq!(
        consts.iter().find(|(n, _)| n == "ENABLE_QUOTE_MINTING").unwrap().1,
        endpoints::ENABLE_QUOTE_MINTING
    );
    let routes = routed(&src);
    assert!(routes.len() >= 37, "parsed only {} routes", routes.len());
    let addr = aggregator().await;

    let mut fails = Vec::new();
    for (name, method) in &routes {
        let path = &consts
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("no endpoint const {name}"))
            .1;
        let none = status(addr, method, path, None).await;
        let wrong = status(addr, method, path, Some("Bearer wrong-token")).await;
        let exempt = path == "/health";
        let ok = exempt || (none == "401" && wrong == "401");
        let line = format!("{method:6} {path} ({name}): no-auth={none} wrong-bearer={wrong}");
        println!("{} {line}{}", if ok { "ok  " } else { "FAIL" }, if exempt { " [exempt]" } else { "" });
        if !ok {
            fails.push(line);
        }
    }
    for path in ["/api-docs/openapi.json", "/swagger-ui/"] {
        println!("info   GET {path}: no-auth={}", status(addr, "GET", path, None).await);
    }
    assert!(
        fails.is_empty(),
        "{} of {} admin-aggregator routes answer without valid credentials:\n{}",
        fails.len(),
        routes.len(),
        fails.join("\n")
    );
}
