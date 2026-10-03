// A request to an admin route (list, lookup, update, enable-minting, ...) must
// never reach the handler through the public listener's address: it is only ever
// served on its own listener, bound separately by `serve_split`.

use std::net::SocketAddr;
use std::time::Duration;

const ADMIN_LIST_PATH: &str = "/admin/quote";

fn free_addr() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("local_addr")
    // listener is dropped here, freeing the port for serve_split to rebind.
}

async fn spawn(web_addr: SocketAddr, admin_addr: SocketAddr) -> tokio::sync::oneshot::Sender<()> {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let ctrl = bcr_wdc_quote_service::test_utils::test_controller();
    let web_router = bcr_wdc_quote_service::web_routes().with_state(ctrl.clone());
    let admin_router = bcr_wdc_quote_service::admin_routes().with_state(ctrl);
    tokio::spawn(async move {
        bcr_wdc_utils::serve::serve_split(web_router, admin_router, web_addr, admin_addr, async {
            let _ = rx.await;
        })
        .await
        .expect("serve_split");
    });
    // give the listeners a moment to bind before the test issues requests.
    tokio::time::sleep(Duration::from_millis(50)).await;
    tx
}

// Invariant: the admin router never answers on the public listener's address.
// Breaks if the merged router is ever served on `bind_address` instead of splitting
// web and admin onto their own listeners.
#[tokio::test]
async fn admin_route_is_unreachable_on_the_public_listener() {
    let web_addr = free_addr();
    let admin_addr = free_addr();
    let _shutdown = spawn(web_addr, admin_addr).await;

    let resp = reqwest::Client::new()
        .get(format!("http://{web_addr}{ADMIN_LIST_PATH}"))
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
}

// Invariant: the admin router does answer on its own listener, so the 404 above is
// isolation and not the admin listener being down entirely.
#[tokio::test]
async fn admin_route_is_reachable_on_the_admin_listener() {
    let web_addr = free_addr();
    let admin_addr = free_addr();
    let _shutdown = spawn(web_addr, admin_addr).await;

    let resp = reqwest::Client::new()
        .get(format!("http://{admin_addr}{ADMIN_LIST_PATH}"))
        .send()
        .await
        .expect("request");

    assert_ne!(resp.status(), reqwest::StatusCode::NOT_FOUND);
}

// Invariant: the public listener keeps serving its own routes after the split.
#[tokio::test]
async fn public_health_is_reachable_on_the_public_listener() {
    let web_addr = free_addr();
    let admin_addr = free_addr();
    let _shutdown = spawn(web_addr, admin_addr).await;

    let resp = reqwest::get(format!("http://{web_addr}/health"))
        .await
        .expect("request");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);
}

// Invariant: the public router never answers on the admin listener's address.
#[tokio::test]
async fn public_health_is_unreachable_on_the_admin_listener() {
    let web_addr = free_addr();
    let admin_addr = free_addr();
    let _shutdown = spawn(web_addr, admin_addr).await;

    let resp = reqwest::get(format!("http://{admin_addr}/health"))
        .await
        .expect("request");

    assert_ne!(resp.status(), reqwest::StatusCode::OK);
}
