use axum::extract::FromRef;
// ----- standard library imports
// ----- extra library imports
use axum::routing::{get, post};
use axum::Router;
// ----- local modules
//mod credit;
mod credit;
mod keys;
mod swap;
mod utils;
// ----- local imports

type TStamp = chrono::DateTime<chrono::Utc>;

#[derive(Clone, FromRef)]
pub struct AppController {
    quote: credit::ProdQuotingService,
}

impl AppController {
    pub fn new(mint_seed: &[u8]) -> Self {
        let quote_keys_repository = credit::persistence::InMemoryQuoteKeysRepository::default();
        let maturing_keys_repository =
            credit::persistence::InMemoryMaturityKeysRepository::default();
        let keys_factory = credit::ProdCreditKeysFactory::new(
            mint_seed,
            quote_keys_repository,
            maturing_keys_repository,
        );
        let quotes_repository = credit::persistence::InMemoryQuoteRepository::default();
        let quotes_factory = credit::ProdQuoteFactory {
            quotes: quotes_repository.clone(),
        };
        let quoting_service = credit::ProdQuotingService {
            keys_gen: keys_factory,
            quotes_gen: quotes_factory,
            quotes: quotes_repository,
        };
        Self {
            quote: quoting_service,
        }
    }
}
pub fn credit_routes(ctrl: AppController) -> Router {
    Router::new()
        .route("/credit/v1/mint/quote", post(credit::web::enquire_quote))
        .route("/credit/v1/mint/quote/:id", get(credit::web::lookup_quote))
        .route(
            "/admin/credit/v1/quote/pending",
            get(credit::admin::list_pending_quotes),
        )
        .route(
            "/admin/credit/v1/quote/accepted",
            get(credit::admin::list_accepted_quotes),
        )
        .route(
            "/admin/credit/v1/quote/:id",
            get(credit::admin::lookup_quote),
        )
        .route(
            "/admin/credit/v1/quote/:id",
            post(credit::admin::resolve_quote),
        )
        .with_state(ctrl)
}
