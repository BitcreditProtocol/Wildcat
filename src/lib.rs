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
pub type ProdKeysRepository = persistence::inmemory::SimpleKeysRepo;
pub type ProdQuoteRepository = persistence::inmemory::QuoteRepo;
pub type ProdSwapProofRepository = persistence::inmemory::ProofRepo;
pub type ProdSwapKeysRepository = persistence::inmemory::SimpleKeysRepo;

pub type ProdCreditKeysFactory = credit::keys::Factory<ProdQuoteKeysRepository, ProdKeysRepository>;
pub type ProdQuoteFactory = credit::quotes::Factory<ProdQuoteRepository>;
pub type ProdSwapCreditKeysRepository = credit::keys::SwapRepository<ProdKeysRepository>;
pub type ProdCreditKeysEnabler =
    credit::keys::KeysEnabler<ProdQuoteKeysRepository, ProdKeysRepository>;

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
        let maturing_keys_repository = ProdKeysRepository::default();
        let keys_factory = ProdCreditKeysFactory::new(
            mint_seed,
            quote_keys_repository,
            maturing_keys_repository.clone(),
        );
        let quotes_repository = ProdQuoteRepository::default();
        let quotes_factory = ProdQuoteFactory {
            quotes: quotes_repository.clone(),
        };
        let quoting_service = ProdQuotingService {
            keys_gen: keys_factory,
            quotes_gen: quotes_factory,
            quotes: quotes_repository,
        };

        let proofs_repository = ProdSwapProofRepository::default();
        let endorsed_keys_repository = ProdKeysRepository::default();
        let debit_keys_repository = ProdKeysRepository::default();
        let credit_keys_repository = credit::keys::SwapRepository {
            endorsed_keys: endorsed_keys_repository.clone(),
            maturing_keys: maturing_keys_repository,
            debit_keys: debit_keys_repository,
        };
        let swap_service = ProdSwapService {
            keys: credit_keys_repository,
            proofs: proofs_repository,
        };
        Self {
            quote: quoting_service,
            swap: swap_service,
        }
    }
}
pub fn credit_routes(ctrl: AppController) -> Router {
    Router::new()
        .route("/v1/swap", post(swap::web::swap_tokens))
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
