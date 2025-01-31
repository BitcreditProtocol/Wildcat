#[tokio::main]
async fn main() {
    let pub_address = std::net::SocketAddr::from(([127, 0, 0, 1], 3338));

    let e = env_logger::Env::new().filter_or("WILDCAT_LOG", "debug");
    env_logger::Builder::from_env(e).init();

    let app = wildcat::AppController::new(&[0u8; 32]);
    let router = wildcat::credit_routes(app);

    axum::Server::bind(&pub_address)
        .serve(router.into_make_service())
        .await
        .unwrap();
}
