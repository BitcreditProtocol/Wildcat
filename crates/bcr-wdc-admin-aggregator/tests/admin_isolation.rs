// core-, quote- and treasury-service serve their admin routes on a listener of their
// own, separate from the public one. The aggregator must reach each admin route
// through the matching `*_admin_url`, and core-service's public routes through
// `core_url`, or its calls land on a listener that does not serve them.

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use axum::{
    extract::State,
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

fn free_addr() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("local_addr")
}

fn url(addr: SocketAddr) -> Url {
    Url::parse(&format!("http://{addr}")).expect("url")
}

fn keyset_info() -> ecash::KeySetInfo {
    ecash::KeySetInfo {
        id: cashu::Id::from_str(KID).expect("kid").into(),
        unit: cl_core::Client::currency_unit(),
        active: true,
        input_fee_ppk: 1,
        final_expiry: None,
    }
}

struct Split {
    web: SocketAddr,
    admin: SocketAddr,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

async fn spawn_split(web: Router, admin: Router) -> Split {
    let (web_addr, admin_addr) = (free_addr(), free_addr());
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        bcr_wdc_utils::serve::serve_split(web, admin, web_addr, admin_addr, async {
            let _ = rx.await;
        })
        .await
        .expect("serve_split");
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    Split {
        web: web_addr,
        admin: admin_addr,
        _shutdown: tx,
    }
}

async fn new_keyset(State(hits): State<Arc<AtomicUsize>>) -> Json<ecash::KeySetInfo> {
    hits.fetch_add(1, Ordering::SeqCst);
    Json(keyset_info())
}

/// core-service with no keysets yet, so the aggregator's pre-flight creates one.
async fn spawn_core(new_keyset_hits: Arc<AtomicUsize>) -> Split {
    let web = Router::new()
        .route(
            cl_core::web_ep::LIST_KEYSET_INFO_V1,
            get(|| async { Json(wire_keys::KeysetInfoListResponse { keysets: vec![] }) }),
        )
        .route(
            cl_core::web_ep::KEYSET_INFO_V1,
            get(|| async { Json(keyset_info()) }),
        );
    let admin = Router::new()
        .route(cl_core::admin_ep::NEW_KEYSET, post(new_keyset))
        .with_state(new_keyset_hits);
    spawn_split(web, admin).await
}

async fn spawn_quote() -> Split {
    let admin = Router::new().route(
        cl_quote::admin_ep::ENABLE_MINTING,
        patch(|| async { Json(wire_quotes::EnableMintingResponse {}) }),
    );
    spawn_split(Router::new(), admin).await
}

async fn spawn_treasury() -> Split {
    let admin = Router::new().route(
        cl_treasury::admin_ep::FOREIGN_BALANCE,
        get(|| async { Json(wire_treasury::ForeignBalanceResponse { balances: vec![] }) }),
    );
    spawn_split(Router::new(), admin).await
}

struct Backends {
    core: Split,
    quote: Split,
    treasury: Split,
    new_keyset_hits: Arc<AtomicUsize>,
}

async fn spawn_backends() -> Backends {
    let new_keyset_hits = Arc::new(AtomicUsize::new(0));
    Backends {
        core: spawn_core(new_keyset_hits.clone()).await,
        quote: spawn_quote().await,
        treasury: spawn_treasury().await,
        new_keyset_hits,
    }
}

fn config(b: &Backends) -> AppConfig {
    let unused = url(free_addr());
    AppConfig {
        core_url: url(b.core.web),
        core_admin_url: url(b.core.admin),
        quotes_admin_url: url(b.quote.admin),
        ebill_url: unused.clone(),
        clowder_url: unused,
        treasury_admin_url: url(b.treasury.admin),
    }
}

async fn aggregator(b: &Backends) -> axum_test::TestServer {
    let ctrl = AppController::new(config(b)).await;
    axum_test::TestServer::new(routes(ctrl)).expect("test server")
}

#[tokio::test]
async fn preflight_creates_keyset_through_core_admin_listener() {
    let b = spawn_backends().await;
    let _server = aggregator(&b).await;
    assert_eq!(b.new_keyset_hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn keyset_info_goes_through_core_public_listener() {
    let b = spawn_backends().await;
    let server = aggregator(&b).await;
    let resp = server
        .get(&endpoints::KEYSET_INFO.replace("{kid}", KID))
        .await;
    resp.assert_status_ok();
}

#[tokio::test]
async fn enable_quote_minting_goes_through_quote_admin_listener() {
    let b = spawn_backends().await;
    let server = aggregator(&b).await;
    let qid = uuid::Uuid::new_v4().to_string();
    let resp = server
        .patch(&endpoints::ENABLE_QUOTE_MINTING.replace("{qid}", &qid))
        .await;
    resp.assert_status_ok();
}

#[tokio::test]
async fn foreign_balance_goes_through_treasury_admin_listener() {
    let b = spawn_backends().await;
    let server = aggregator(&b).await;
    let resp = server.get(endpoints::FOREIGN_BALANCE).await;
    resp.assert_status_ok();
}
