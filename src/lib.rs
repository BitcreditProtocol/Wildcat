use axum::extract::FromRef;
// ----- standard library imports
// ----- extra library imports
use axum::routing::{get, post};
use axum::Router;
// ----- local modules
//mod credit;
mod credit;
mod keys;
mod persistence;
mod swap;
mod utils;
// ----- local imports

type TStamp = chrono::DateTime<chrono::Utc>;

pub type ProdQuoteKeysRepository = persistence::inmemory::KeysetIDQuoteIDMap;
pub type ProdMaturityKeysRepository = persistence::inmemory::KeysetIDEntryMap;
pub type ProdQuoteRepository = persistence::inmemory::QuotesIDMap;

pub type ProdCreditKeysFactory =
    credit::keys::Factory<ProdQuoteKeysRepository, ProdMaturityKeysRepository>;
pub type ProdQuoteFactory = credit::quotes::Factory<ProdQuoteRepository>;
pub type ProdQuotingService = credit::quotes::Service<ProdCreditKeysFactory, ProdQuoteRepository>;

//pub type ProdCreditKeysRepository = crate::credit::keys::SwapRepository<crate::>
//pub type ProdCreditSwapKeysRepository = crate::credit::keys::SwapRepository<>;
//pub type ProdSwapService = swap::service::Service<>;

#[derive(Clone, FromRef)]
pub struct AppController {
    quote: ProdQuotingService,
}

impl AppController {
    pub fn new(mint_seed: &[u8]) -> Self {
        let quote_keys_repository = ProdQuoteKeysRepository::default();
        let maturing_keys_repository = ProdMaturityKeysRepository::default();
        let keys_factory =
            ProdCreditKeysFactory::new(mint_seed, quote_keys_repository, maturing_keys_repository);
        let quotes_repository = ProdQuoteRepository::default();
        let quotes_factory = ProdQuoteFactory {
            quotes: quotes_repository.clone(),
        };
        let quoting_service = ProdQuotingService {
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
