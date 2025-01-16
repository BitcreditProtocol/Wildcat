// ----- standard library imports
// ----- extra library imports
// ----- local modules
mod credit;
// ----- local imports

#[tokio::main]
async fn main() {
    let pub_address = std::net::SocketAddr::from(([127, 0, 0, 1], 3338));

    let e = env_logger::Env::new().filter_or("WILDCAT_LOG", "debug");
    env_logger::Builder::from_env(e).init();

    let mint_quote_repo = credit::persistence::InMemoryQuoteRepository::default();

    let mint_service = credit::mint::Service {
        quotes: mint_quote_repo,
    };

    let controller = credit::Controller {
        quote_service: mint_service,
    };

    let credit_route = credit::pub_routes(controller.clone());
    let admin_route = credit::admin_routes(controller.clone());
    let app = axum::Router::new().merge(credit_route).merge(admin_route);
    axum::Server::bind(&pub_address)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
