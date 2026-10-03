// ----- standard library imports
use std::future::Future;
use std::net::SocketAddr;
// ----- extra library imports
use axum::Router;
use futures::FutureExt;
// ----- end imports

/// Serves `web` and `admin` on two independent listeners, each bound to its own
/// address, sharing one graceful-shutdown signal between them.
///
/// A service that merges admin-only routes into the same router as its public web
/// routes exposes them on whatever address that router is served on. Binding each
/// router to its own listener keeps the admin routes reachable only on `admin_addr`,
/// which a deployer can bind to loopback or an unpublished interface.
pub async fn serve_split<F>(
    web: Router,
    admin: Router,
    web_addr: SocketAddr,
    admin_addr: SocketAddr,
    shutdown: F,
) -> std::io::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let shutdown = shutdown.shared();

    let web_listener = tokio::net::TcpListener::bind(web_addr).await?;
    let admin_listener = tokio::net::TcpListener::bind(admin_addr).await?;

    let web_shutdown = shutdown.clone();
    let web_server = axum::serve(web_listener, web)
        .with_graceful_shutdown(async move { web_shutdown.await });

    let admin_server = axum::serve(admin_listener, admin)
        .with_graceful_shutdown(async move { shutdown.await });

    tokio::try_join!(web_server, admin_server)?;
    Ok(())
}
