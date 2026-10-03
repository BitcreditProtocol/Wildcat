#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    body::Body,
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::IntoResponse,
    Router,
};
use bcr_common::client::Url;

#[derive(Clone, Debug)]
pub struct Hit {
    pub listener: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub unserved: bool,
    pub body: String,
}

#[derive(Clone, Default)]
pub struct Log(pub Arc<Mutex<Vec<Hit>>>);

impl Log {
    pub fn hits(&self) -> Vec<Hit> {
        self.0.lock().unwrap().clone()
    }
    pub fn clear(&self) {
        self.0.lock().unwrap().clear()
    }
    pub fn unserved(&self) -> Vec<Hit> {
        self.hits().into_iter().filter(|h| h.unserved).collect()
    }
    pub fn dump(&self) -> String {
        self.hits()
            .iter()
            .map(|h| {
                format!(
                    "  [{}] {} {} -> {}{}",
                    h.listener,
                    h.method,
                    h.path,
                    h.status,
                    if h.unserved { "  (NO ROUTE ON THIS LISTENER)" } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

const UNSERVED: &str = "x-probe-unserved";

async fn unmatched() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, [(UNSERVED, "1")])
}

async fn wrong_method() -> impl IntoResponse {
    (StatusCode::METHOD_NOT_ALLOWED, [(UNSERVED, "1")])
}

/// Records every request reaching `router` and whether the router had a route
/// (path + method) for it.
pub fn tap(router: Router, name: &str, log: &Log) -> Router {
    let name = name.to_string();
    let log = log.clone();
    router
        .fallback(unmatched)
        .method_not_allowed_fallback(wrong_method)
        .layer(middleware::from_fn(move |req: Request, next: Next| {
            let name = name.clone();
            let log = log.clone();
            async move {
                let method = req.method().to_string();
                let path = req.uri().path().to_string();
                let (parts, body) = req.into_parts();
                let bytes = axum::body::to_bytes(body, usize::MAX)
                    .await
                    .unwrap_or_default();
                let body_str = String::from_utf8_lossy(&bytes).to_string();
                let req = Request::from_parts(parts, Body::from(bytes));
                let resp = next.run(req).await;
                let unserved = resp.headers().contains_key(UNSERVED);
                log.0.lock().unwrap().push(Hit {
                    listener: name,
                    method,
                    path,
                    status: resp.status().as_u16(),
                    unserved,
                    body: body_str,
                });
                resp
            }
        }))
}

pub fn free_addr() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("local_addr")
}

pub fn url(addr: SocketAddr) -> Url {
    Url::parse(&format!("http://{addr}")).expect("url")
}

pub struct Split {
    pub web: Url,
    pub admin: Url,
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

/// Real `serve_split` on two real TCP sockets.
pub async fn spawn_split(web: Router, admin: Router) -> Split {
    let (web_addr, admin_addr) = (free_addr(), free_addr());
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        bcr_wdc_utils::serve::serve_split(web, admin, web_addr, admin_addr, async {
            let _ = rx.await;
        })
        .await
        .expect("serve_split");
    });
    wait_up(web_addr).await;
    wait_up(admin_addr).await;
    Split {
        web: url(web_addr),
        admin: url(admin_addr),
        _shutdown: tx,
    }
}

pub struct Single {
    pub url: Url,
    _handle: tokio::task::JoinHandle<()>,
}

pub async fn spawn_single(router: Router) -> Single {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve");
    });
    wait_up(addr).await;
    Single {
        url: url(addr),
        _handle: handle,
    }
}

async fn wait_up(addr: SocketAddr) {
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{addr} never came up");
}

pub async fn core_split(log: &Log) -> (Split, bcr_wdc_core_service::AppController) {
    let ctrl = bcr_wdc_core_service::test_utils::test_controller();
    let web = tap(
        bcr_wdc_core_service::web_routes::<bcr_wdc_core_service::AppController>()
            .with_state(ctrl.clone()),
        "core.web",
        log,
    );
    let admin = tap(
        bcr_wdc_core_service::admin_routes::<bcr_wdc_core_service::AppController>()
            .with_state(ctrl.clone()),
        "core.admin",
        log,
    );
    (spawn_split(web, admin).await, ctrl)
}

pub async fn mint_split(log: &Log) -> Split {
    let ctrl = bcr_wdc_mint_service::test_utils::test_controller();
    let web = tap(
        bcr_wdc_mint_service::web_routes::<bcr_wdc_mint_service::AppController>()
            .with_state(ctrl.clone()),
        "mint.web",
        log,
    );
    let admin = tap(
        bcr_wdc_mint_service::admin_routes::<bcr_wdc_mint_service::AppController>()
            .with_state(ctrl),
        "mint.admin",
        log,
    );
    spawn_split(web, admin).await
}

pub async fn treasury_split(log: &Log) -> Split {
    let ctrl = bcr_wdc_treasury_service::test_utils::test_controller().await;
    let web = tap(
        bcr_wdc_treasury_service::web_routes::<bcr_wdc_treasury_service::AppController>()
            .with_state(ctrl.clone()),
        "treasury.web",
        log,
    );
    let admin = tap(
        bcr_wdc_treasury_service::admin_routes::<bcr_wdc_treasury_service::AppController>()
            .with_state(ctrl),
        "treasury.admin",
        log,
    );
    spawn_split(web, admin).await
}

/// Base topology: treasury's single merged router (what `treasury_url` used to reach).
pub async fn treasury_merged(log: &Log) -> Single {
    let ctrl = bcr_wdc_treasury_service::test_utils::test_controller().await;
    spawn_single(tap(
        bcr_wdc_treasury_service::routes(ctrl),
        "treasury.merged(base)",
        log,
    ))
    .await
}

pub async fn quote_split(log: &Log) -> Split {
    let ctrl = bcr_wdc_quote_service::test_utils::test_controller();
    let web = tap(
        bcr_wdc_quote_service::web_routes::<bcr_wdc_quote_service::AppController>()
            .with_state(ctrl.clone()),
        "quote.web",
        log,
    );
    let admin = tap(
        bcr_wdc_quote_service::admin_routes::<bcr_wdc_quote_service::AppController>()
            .with_state(ctrl),
        "quote.admin",
        log,
    );
    spawn_split(web, admin).await
}

/// A backend with no routes at all (ebill-service, clowder): records and 404s.
pub async fn stub(name: &str, log: &Log) -> Single {
    spawn_single(tap(Router::new(), name, log)).await
}
