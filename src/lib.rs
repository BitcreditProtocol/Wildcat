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

pub type ProdQuoteKeysRepository = persistence::inmemory::QuoteKeysRepo;
pub type ProdEndorsedKeysRepository = persistence::inmemory::SimpleKeysRepo;
pub type ProdMaturityKeysRepository = persistence::inmemory::SimpleKeysRepo;
pub type ProdDebitKeysRepository = persistence::inmemory::SimpleKeysRepo;
pub type ProdQuoteRepository = persistence::inmemory::QuoteRepo;
pub type ProdSwapProofRepository = persistence::inmemory::ProofRepo;
pub type ProdSwapKeysRepository = persistence::inmemory::SimpleKeysRepo;

pub type ProdCreditKeysFactory =
    credit::keys::Factory<ProdQuoteKeysRepository, ProdMaturityKeysRepository>;
pub type ProdQuoteFactory = credit::quotes::Factory<ProdQuoteRepository>;
pub type ProdSwapCreditKeysRepository = credit::keys::SwapRepository<
    ProdEndorsedKeysRepository,
    ProdMaturityKeysRepository,
    ProdDebitKeysRepository,
>;
pub type ProdQuotingService = credit::quotes::Service<ProdCreditKeysFactory, ProdQuoteRepository>;

pub type ProdSwapService = swap::Service<ProdSwapCreditKeysRepository, ProdSwapProofRepository>;

#[derive(Clone, FromRef)]
pub struct AppController {
    quote: ProdQuotingService,
    swap: ProdSwapService,
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
