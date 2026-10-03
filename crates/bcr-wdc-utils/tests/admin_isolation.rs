// A request authenticated with `require_api_key` must never reach the protected
// handler unless it carries the exact configured bearer token, and a router served
// through `serve_split` must never answer for a listener it was not bound to.

use std::net::SocketAddr;
use std::time::Duration;

use axum::{routing::get, Router};
use bcr_wdc_utils::{auth::require_api_key, serve::serve_split};

const SECRET: &str = "s3cr3t-token";

fn free_addr() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("local_addr")
    // listener is dropped here, freeing the port for serve_split to rebind.
}

fn web_router() -> Router {
    Router::new().route("/web-only", get(|| async { "web" }))
}

fn admin_router() -> Router {
    let admin = Router::new().route("/admin-only", get(|| async { "admin" }));
    require_api_key(admin, SECRET)
}

async fn spawn(web_addr: SocketAddr, admin_addr: SocketAddr) -> tokio::sync::oneshot::Sender<()> {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        serve_split(web_router(), admin_router(), web_addr, admin_addr, async {
            let _ = rx.await;
        })
        .await
        .expect("serve_split");
    });
    // give the listeners a moment to bind before the test issues requests.
    tokio::time::sleep(Duration::from_millis(50)).await;
    tx
}

// Invariant: a request with no Authorization header never reaches the protected
// handler. Breaks if a missing header is treated as an implicit allow (200 instead
// of 401).
#[tokio::test]
async fn missing_authorization_header_is_rejected() {
    let web_addr = free_addr();
    let admin_addr = free_addr();
    let _shutdown = spawn(web_addr, admin_addr).await;

    let resp = reqwest::get(format!("http://{admin_addr}/admin-only"))
        .await
        .expect("request");

    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

// Invariant: a request bearing a token that does not match the configured secret
// never reaches the protected handler. Breaks if the comparison accepts any
// non-empty token, or a prefix/suffix match, instead of an exact match.
#[tokio::test]
async fn wrong_token_is_rejected() {
    let web_addr = free_addr();
    let admin_addr = free_addr();
    let _shutdown = spawn(web_addr, admin_addr).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{admin_addr}/admin-only"))
        .bearer_auth("not-the-secret")
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

// Invariant: a request bearing exactly the configured secret reaches the protected
// handler. Breaks if the middleware rejects every request regardless of the token,
// masking the two rejection cases above as trivially true.
#[tokio::test]
async fn correct_token_is_allowed() {
    let web_addr = free_addr();
    let admin_addr = free_addr();
    let _shutdown = spawn(web_addr, admin_addr).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{admin_addr}/admin-only"))
        .bearer_auth(SECRET)
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "admin");
}

// Invariant: the web listener never serves the admin router's routes, and the admin
// listener never serves the web router's routes. Breaks if `serve_split` merges both
// routers onto one listener instead of keeping them on separate sockets.
#[tokio::test]
async fn listeners_never_serve_each_others_router() {
    let web_addr = free_addr();
    let admin_addr = free_addr();
    let _shutdown = spawn(web_addr, admin_addr).await;

    let admin_route_on_web_listener = reqwest::get(format!("http://{web_addr}/admin-only"))
        .await
        .expect("request");
    assert_eq!(
        admin_route_on_web_listener.status(),
        reqwest::StatusCode::NOT_FOUND
    );

    // The admin router's auth layer runs before routing, so an unmatched path on the
    // admin listener answers 401 rather than 404; either way it must never be the
    // web router's 200 response.
    let web_route_on_admin_listener = reqwest::get(format!("http://{admin_addr}/web-only"))
        .await
        .expect("request");
    assert_ne!(
        web_route_on_admin_listener.status(),
        reqwest::StatusCode::OK
    );

    // and each router does answer its own route, so the 404s above are isolation,
    // not a listener that is not actually up.
    let web_on_web = reqwest::get(format!("http://{web_addr}/web-only"))
        .await
        .expect("request");
    assert_eq!(web_on_web.status(), reqwest::StatusCode::OK);
}
