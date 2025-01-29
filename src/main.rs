use wildcat::credit;

#[tokio::main]
async fn main() {
    let pub_address = std::net::SocketAddr::from(([127, 0, 0, 1], 3338));

    let e = env_logger::Env::new().filter_or("WILDCAT_LOG", "debug");
    env_logger::Builder::from_env(e).init();

    let quote_repo = credit::persistence::InMemoryQuoteRepository::default();
    let quote_keys_repo = credit::persistence::InMemoryKeysRepository::default();
    let maturing_keys_repo = credit::persistence::InMemoryKeysRepository::default();
    let ctrl = credit::Controller::new(
        &[0u8; 32],
        quote_repo.clone(),
        quote_keys_repo,
        maturing_keys_repo,
    );

    let credit_route = credit::web::routes(ctrl.clone());
    let admin_route = credit::admin::routes(ctrl.clone());
    let app = axum::Router::new().merge(credit_route).merge(admin_route);

    axum::Server::bind(&pub_address)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
