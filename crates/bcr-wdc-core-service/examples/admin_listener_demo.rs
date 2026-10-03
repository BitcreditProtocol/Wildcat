//! Boots the public and admin routers on two separate local listeners, backed by
//! the in-memory dummy controller used in tests, so `scripts/check_admin_isolation.sh`
//! can curl both live and confirm the admin surface is unreachable from the public one.

#[cfg(feature = "test-utils")]
#[tokio::main]
async fn main() {
    use bcr_wdc_core_service::test_utils::test_controller;

    let web_addr: std::net::SocketAddr = "127.0.0.1:18338".parse().unwrap();
    let admin_addr: std::net::SocketAddr = "127.0.0.1:18339".parse().unwrap();

    let ctrl = test_controller();
    let web_router = bcr_wdc_core_service::web_routes().with_state(ctrl.clone());
    let admin_router = bcr_wdc_core_service::admin_routes().with_state(ctrl);

    eprintln!("web listening on {web_addr}");
    eprintln!("admin listening on {admin_addr}");

    bcr_wdc_utils::serve::serve_split(
        web_router,
        admin_router,
        web_addr,
        admin_addr,
        std::future::pending(),
    )
    .await
    .expect("serve_split");
}

#[cfg(not(feature = "test-utils"))]
fn main() {
    panic!("run with --features test-utils");
}
