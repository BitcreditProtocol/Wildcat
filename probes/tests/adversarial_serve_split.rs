//! Lifecycle probes of `bcr_wdc_utils::serve::serve_split` on real sockets:
//! bind failures, shared addresses, graceful shutdown, restart, load, cancellation.
mod adversarial_common;

use std::net::SocketAddr;
use std::time::Duration;

use adversarial_common::{accepts, free_addr, raw, wait_listening};
use axum::{routing::get, routing::post, Router};
use bcr_common::client::admin::core::admin_ep as core_admin;
use bcr_wdc_utils::serve::serve_split;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;

fn slow(tag: &'static str) -> Router {
    Router::new()
        .route("/", get(move || async move { tag }))
        .route(
            "/slow",
            get(move || async move {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                tag
            }),
        )
        .route("/body", post(move |b: String| async move { format!("{tag}:{}", b.len()) }))
}

type Run = (tokio::task::JoinHandle<std::io::Result<()>>, oneshot::Sender<()>);

fn spawn(w: SocketAddr, a: SocketAddr) -> Run {
    let (tx, rx) = oneshot::channel::<()>();
    let h = tokio::spawn(serve_split(slow("WEB"), slow("ADMIN"), w, a, async move {
        let _ = rx.await;
    }));
    (h, tx)
}

fn get_req(path: &str) -> Vec<u8> {
    format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").into_bytes()
}

async fn finished(h: &mut tokio::task::JoinHandle<std::io::Result<()>>, within: Duration) -> Option<std::io::Result<()>> {
    tokio::time::timeout(within, h).await.ok().map(|r| r.expect("serve_split panicked"))
}

#[tokio::test]
async fn admin_addr_taken_errs_and_leaves_web_unbound() {
    let (w, a) = (free_addr("127.0.0.1"), free_addr("127.0.0.1"));
    let _squatter = std::net::TcpListener::bind(a).unwrap();
    let (mut h, _tx) = spawn(w, a);
    let res = finished(&mut h, Duration::from_secs(2)).await;
    assert!(matches!(res, Some(Err(_))), "serve_split did not fail on a taken admin address: {res:?}");
    assert!(!accepts(w).await, "web address still accepting after serve_split returned Err");
    std::net::TcpListener::bind(w).expect("web address cannot be re-bound after the failed start");
}

#[tokio::test]
async fn web_addr_taken_errs_and_leaves_admin_unbound() {
    let (w, a) = (free_addr("127.0.0.1"), free_addr("127.0.0.1"));
    let _squatter = std::net::TcpListener::bind(w).unwrap();
    let (mut h, _tx) = spawn(w, a);
    let res = finished(&mut h, Duration::from_secs(2)).await;
    assert!(matches!(res, Some(Err(_))), "serve_split did not fail on a taken web address: {res:?}");
    assert!(!accepts(a).await, "admin address accepting after serve_split returned Err");
}

#[tokio::test]
async fn same_addr_for_web_and_admin_errs() {
    let w = free_addr("127.0.0.1");
    let (mut h, _tx) = spawn(w, w);
    let res = finished(&mut h, Duration::from_secs(2)).await;
    assert!(matches!(res, Some(Err(_))), "web == admin did not fail: {res:?}");
    assert!(!accepts(w).await, "address still accepting after the failed start");
}

/// web on the wildcard, admin on loopback, same port (and the reverse).
/// Either must fail; if one started, loopback callers would reach the admin
/// router on the port the deployer believes is the public one.
#[tokio::test]
async fn wildcard_vs_loopback_same_port() {
    for (wip, aip) in [("0.0.0.0", "127.0.0.1"), ("127.0.0.1", "0.0.0.0")] {
        let port = free_addr("0.0.0.0").port();
        let w: SocketAddr = format!("{wip}:{port}").parse().unwrap();
        let a: SocketAddr = format!("{aip}:{port}").parse().unwrap();
        let (mut h, tx) = spawn(w, a);
        let res = finished(&mut h, Duration::from_secs(1)).await;
        println!("web {w} admin {a} -> {res:?}");
        if res.is_none() {
            let (_, body) = raw(format!("127.0.0.1:{port}").parse().unwrap(), &get_req("/")).await;
            let _ = tx.send(());
            panic!("web {w} + admin {a} both bound; 127.0.0.1:{port} answered {:?}", body.lines().last());
        }
        assert!(matches!(res, Some(Err(_))));
    }
}

/// A second run on the same addresses while the first is serving must fail and
/// must not disturb the first.
#[tokio::test]
async fn two_runs_at_once() {
    let (w, a) = (free_addr("127.0.0.1"), free_addr("127.0.0.1"));
    let (mut h1, tx1) = spawn(w, a);
    wait_listening(w).await;
    wait_listening(a).await;
    let (mut h2, _tx2) = spawn(w, a);
    let r2 = finished(&mut h2, Duration::from_secs(2)).await;
    assert!(matches!(r2, Some(Err(_))), "second run did not fail: {r2:?}");
    assert!(raw(w, &get_req("/")).await.1.ends_with("WEB"));
    assert!(raw(a, &get_req("/")).await.1.ends_with("ADMIN"));
    tx1.send(()).unwrap();
    assert!(matches!(finished(&mut h1, Duration::from_secs(5)).await, Some(Ok(()))));
}

/// Shutdown: both listeners stop accepting at once; in-flight slow requests on
/// each complete; then the same two addresses can be bound again immediately.
#[tokio::test]
async fn graceful_shutdown_then_restart() {
    let (w, a) = (free_addr("127.0.0.1"), free_addr("127.0.0.1"));
    let (mut h, tx) = spawn(w, a);
    wait_listening(w).await;
    wait_listening(a).await;
    let web_slow = tokio::spawn(async move { raw(w, &get_req("/slow")).await });
    let admin_slow = tokio::spawn(async move { raw(a, &get_req("/slow")).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    tx.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!accepts(w).await, "web still accepting after shutdown");
    assert!(!accepts(a).await, "admin still accepting after shutdown");
    let (ws, wb) = web_slow.await.unwrap();
    let (as_, ab) = admin_slow.await.unwrap();
    assert_eq!((ws, as_), (200, 200), "in-flight requests cut: {wb:?} {ab:?}");
    assert!(wb.ends_with("WEB") && ab.ends_with("ADMIN"));
    let res = finished(&mut h, Duration::from_secs(5)).await;
    assert!(matches!(res, Some(Ok(()))), "serve_split did not finish after shutdown: {res:?}");

    let (mut h2, tx2) = spawn(w, a);
    tokio::time::sleep(Duration::from_millis(200)).await;
    if h2.is_finished() {
        panic!("restart on the same addresses failed: {:?}", finished(&mut h2, Duration::from_secs(1)).await);
    }
    assert!(raw(w, &get_req("/")).await.1.ends_with("WEB"));
    assert!(raw(a, &get_req("/")).await.1.ends_with("ADMIN"));
    tx2.send(()).unwrap();
    assert!(matches!(finished(&mut h2, Duration::from_secs(5)).await, Some(Ok(()))));
}

/// An idle keep-alive connection on the admin port must not survive shutdown.
#[tokio::test]
async fn idle_keepalive_closed_on_shutdown() {
    let (w, a) = (free_addr("127.0.0.1"), free_addr("127.0.0.1"));
    let (mut h, tx) = spawn(w, a);
    wait_listening(a).await;
    let mut s = TcpStream::connect(a).await.unwrap();
    s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    let mut buf = [0u8; 512];
    let n = s.read(&mut buf).await.unwrap();
    assert!(String::from_utf8_lossy(&buf[..n]).ends_with("ADMIN"));
    tx.send(()).unwrap();
    let res = finished(&mut h, Duration::from_secs(5)).await;
    assert!(matches!(res, Some(Ok(()))), "shutdown hung on an idle keep-alive: {res:?}");
    let _ = s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").await;
    let n = tokio::time::timeout(Duration::from_secs(2), s.read(&mut buf)).await.unwrap().unwrap_or(0);
    assert_eq!(n, 0, "admin keep-alive still served after shutdown: {:?}", String::from_utf8_lossy(&buf[..n]));
}

/// Base topology: one `axum::serve` of the merged router with graceful shutdown, as
/// every service's main.rs ran it before the split.
fn spawn_base(w: SocketAddr) -> Run {
    let (tx, rx) = oneshot::channel::<()>();
    let h = tokio::spawn(async move {
        let l = tokio::net::TcpListener::bind(w).await?;
        axum::serve(l, slow("WEB").merge(Router::new().route("/admin", get(|| async { "ADMIN" }))))
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            })
            .await
    });
    (h, tx)
}

/// Opens `stall` on the public port, signals shutdown, and reports whether the
/// server finished within `within`.
async fn shutdown_with_stall(h: &mut tokio::task::JoinHandle<std::io::Result<()>>, tx: oneshot::Sender<()>, w: SocketAddr, stall: &[u8], within: Duration) -> bool {
    wait_listening(w).await;
    let mut s = TcpStream::connect(w).await.unwrap();
    s.write_all(stall).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    tx.send(()).unwrap();
    let done = finished(h, within).await.is_some();
    drop(s);
    done
}

/// A stalled public request must not hold serve_split past shutdown any longer than it
/// held the base's single server.
async fn stall_no_worse_than_base(stall: &[u8]) {
    let within = Duration::from_secs(5);
    let w = free_addr("127.0.0.1");
    let (mut h, tx) = spawn_base(w);
    let base = shutdown_with_stall(&mut h, tx, w, stall, within).await;
    h.abort();
    let (w, a) = (free_addr("127.0.0.1"), free_addr("127.0.0.1"));
    let (mut h, tx) = spawn(w, a);
    let split = shutdown_with_stall(&mut h, tx, w, stall, within).await;
    h.abort();
    println!("{:?}: base finished={base} split finished={split} within {within:?}", String::from_utf8_lossy(stall));
    assert!(split || !base, "base shut down despite the stall, serve_split did not");
}

#[tokio::test]
async fn public_slow_body_blocks_shutdown_no_worse_than_base() {
    stall_no_worse_than_base(b"POST /body HTTP/1.1\r\nHost: x\r\nContent-Length: 1000\r\n\r\nabc").await;
}

#[tokio::test]
async fn public_partial_headers_block_shutdown_no_worse_than_base() {
    stall_no_worse_than_base(b"GET / HTTP/1.1\r\nHost: x\r\nX-Slow: ").await;
}

/// While an admin request is in flight after shutdown, the web listener must
/// already be closed (each server stops accepting independently).
#[tokio::test]
async fn web_stops_while_admin_drains() {
    let (w, a) = (free_addr("127.0.0.1"), free_addr("127.0.0.1"));
    let (mut h, tx) = spawn(w, a);
    wait_listening(w).await;
    wait_listening(a).await;
    let admin_slow = tokio::spawn(async move { raw(a, &get_req("/slow")).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    tx.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!h.is_finished());
    assert!(!accepts(w).await, "web accepting while admin drains");
    assert_eq!(admin_slow.await.unwrap().0, 200);
    assert!(matches!(finished(&mut h, Duration::from_secs(5)).await, Some(Ok(()))));
}

/// Dropping the serve_split future (task aborted) closes both listeners. Documents
/// what happens to a keep-alive admin connection opened before.
#[tokio::test]
async fn aborted_run_listeners_closed_and_rebindable() {
    let (w, a) = (free_addr("127.0.0.1"), free_addr("127.0.0.1"));
    let (h, _tx) = spawn(w, a);
    wait_listening(a).await;
    let mut s = TcpStream::connect(a).await.unwrap();
    s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    let mut buf = [0u8; 512];
    let _ = s.read(&mut buf).await.unwrap();
    h.abort();
    let _ = h.await;
    assert!(!accepts(w).await && !accepts(a).await, "listeners still accepting after abort");
    let _ = s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").await;
    let n = tokio::time::timeout(Duration::from_secs(2), s.read(&mut buf)).await.map(|r| r.unwrap_or(0)).unwrap_or(0);
    println!(
        "keep-alive admin connection after abort answered: {:?}",
        String::from_utf8_lossy(&buf[..n]).lines().last()
    );
    let (h2, tx2) = spawn(w, a);
    wait_listening(w).await;
    wait_listening(a).await;
    tx2.send(()).unwrap();
    assert!(matches!(h2.await.unwrap(), Ok(())));
}

/// 500 concurrent requests across both listeners of the real core-service split.
#[tokio::test]
async fn core_500_concurrent_requests_split_correctly() {
    let (w, a) = (free_addr("127.0.0.1"), free_addr("127.0.0.1"));
    let c = bcr_wdc_core_service::test_utils::test_controller();
    let (tx, rx) = oneshot::channel::<()>();
    let h = tokio::spawn(serve_split(
        bcr_wdc_core_service::web_routes().with_state(c.clone()),
        bcr_wdc_core_service::admin_routes().with_state(c),
        w,
        a,
        async move {
            let _ = rx.await;
        },
    ));
    wait_listening(w).await;
    wait_listening(a).await;
    let cl = reqwest::Client::new();
    let sign = |addr: SocketAddr| {
        cl.post(format!("http://{addr}{}", core_admin::SIGN))
            .header("content-type", "application/json")
            .body("{}")
            .send()
    };
    let control = sign(a).await.unwrap().status().as_u16();
    assert!(control != 404);
    let mut futs = vec![];
    for i in 0..500 {
        let cl = cl.clone();
        futs.push(async move {
            let (addr, path, expect) = match i % 3 {
                0 => (w, "/health".to_string(), 200),
                1 => (w, core_admin::SIGN.to_string(), 404),
                _ => (a, core_admin::SIGN.to_string(), control),
            };
            let r = if path == "/health" {
                cl.get(format!("http://{addr}{path}")).send().await
            } else {
                cl.post(format!("http://{addr}{path}"))
                    .header("content-type", "application/json")
                    .body("{}")
                    .send()
                    .await
            };
            match r {
                Ok(r) if r.status().as_u16() == expect => None,
                Ok(r) => Some(format!("{i} {addr}{path}: {} != {expect}", r.status())),
                Err(e) => Some(format!("{i} {addr}{path}: {e}")),
            }
        });
    }
    let bad: Vec<_> = futures::future::join_all(futs).await.into_iter().flatten().collect();
    tx.send(()).unwrap();
    assert!(matches!(h.await.unwrap(), Ok(())));
    assert!(bad.is_empty(), "{} of 500 wrong: {:?}", bad.len(), &bad[..bad.len().min(10)]);
}
