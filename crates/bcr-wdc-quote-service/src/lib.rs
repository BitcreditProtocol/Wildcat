use axum::extract::FromRef;
// ----- standard library imports
// ----- extra library imports
use axum::routing::{get, post};
use axum::Router;
use bcr_wdc_keys as keys;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut12 as cdk12;
use utoipa::OpenApi;
// ----- local modules
//mod credit;
mod credit;
mod persistence;
mod swap;
mod utils;
// ----- local imports

type TStamp = chrono::DateTime<chrono::Utc>;

pub type ProdQuoteKeysRepository = persistence::surreal::keysets::QuoteKeysDB;
pub type ProdKeysRepository = persistence::surreal::keysets::KeysDB;
pub type ProdActiveKeysRepository = persistence::surreal::keysets::KeysDB;
pub type ProdQuoteRepository = persistence::surreal::quotes::DB;
pub type ProdProofRepository = persistence::surreal::proofs::DB;

pub type ProdCreditKeysFactory = credit::keys::Factory<ProdQuoteKeysRepository, ProdKeysRepository>;
pub type ProdQuotingService = credit::quotes::Service<ProdCreditKeysFactory, ProdQuoteRepository>;

pub type ProdCreditKeysRepository =
    crate::credit::keys::SwapRepository<ProdKeysRepository, ProdActiveKeysRepository>;
pub type ProdSwapService = swap::Service<ProdCreditKeysRepository, ProdProofRepository>;

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AppConfig {
    dbs: persistence::surreal::DBConfig,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    quote: ProdQuotingService,
    swap: ProdSwapService,
}

impl AppController {
    pub async fn new(mint_seed: &[u8], cfg: AppConfig) -> Self {
        let AppConfig { dbs, .. } = cfg;
        let persistence::surreal::DBConfig {
            quotes,
            quotes_keys,
            maturity_keys,
            endorsed_keys,
            debit_keys,
            proofs,
            ..
        } = dbs;
        let quotes_repository = ProdQuoteRepository::new(quotes)
            .await
            .expect("DB connection to quotes failed");
        let quote_keys_repository = ProdQuoteKeysRepository::new(quotes_keys)
            .await
            .expect("DB connection to quoteskeys failed");
        let endorsed_keys_repository = ProdKeysRepository::new(endorsed_keys)
            .await
            .expect("DB connection to endorsed_keys failed");
        let maturity_keys_repository = ProdKeysRepository::new(maturity_keys)
            .await
            .expect("DB connection to maturity_keys failed");
        let debit_keys_repository = ProdActiveKeysRepository::new(debit_keys)
            .await
            .expect("DB connection to debit_keys failed");
        let proofs_repo = ProdProofRepository::new(proofs)
            .await
            .expect("DB connection to proofs failed");

        let keys_factory = ProdCreditKeysFactory::new(
            mint_seed,
            quote_keys_repository,
            maturity_keys_repository.clone(),
        );
        let quoting_service = ProdQuotingService {
            keys_gen: keys_factory,
            quotes: quotes_repository,
        };

        let credit_keys_for_swaps = ProdCreditKeysRepository {
            debit_keys: debit_keys_repository,
            endorsed_keys: endorsed_keys_repository,
            maturity_keys: maturity_keys_repository,
        };
        let swaps = ProdSwapService {
            keys: credit_keys_for_swaps,
            proofs: proofs_repo,
        };
        Self {
            quote: quoting_service,
            swap: swaps,
        }
    }
}
pub fn credit_routes(ctrl: AppController) -> Router {
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    Router::new()
        .route("/v1/swap", post(swap::web::swap_tokens))
        .route("/v1/credit/mint/quote", post(credit::web::enquire_quote))
        .route("/v1/credit/mint/quote/:id", get(credit::web::lookup_quote))
        .route(
            "/v1/credit/mint/quote/:id",
            post(credit::web::resolve_offer),
        )
        .route(
            "/v1/admin/credit/quote/pending",
            get(credit::admin::list_pending_quotes),
        )
        .route(
            "/v1/admin/credit/quote/accepted",
            get(credit::admin::list_accepted_quotes),
        )
        .route(
            "/v1/admin/credit/quote/:id",
            get(credit::admin::lookup_quote),
        )
        .route(
            "/v1/admin/credit/quote/:id",
            post(credit::admin::resolve_quote),
        )
        .with_state(ctrl)
        .merge(swagger)
}

#[derive(utoipa::OpenApi)]
#[openapi(
    components(schemas(
        //bcr_ebill_core::contact::IdentityPublicData,
        bcr_wdc_webapi::quotes::BillInfo,
        bcr_wdc_webapi::quotes::EnquireReply,
        bcr_wdc_webapi::quotes::EnquireRequest,
        bcr_wdc_webapi::quotes::InfoReply,
        bcr_wdc_webapi::quotes::ListReply,
        bcr_wdc_webapi::quotes::ResolveOffer,
        bcr_wdc_webapi::quotes::ResolveRequest,
        bcr_wdc_webapi::quotes::StatusReply,
        cashu::Amount,
        cdk00::BlindSignature,
        cdk00::BlindedMessage,
        cdk00::Witness,
        cdk12::BlindSignatureDleq,
    ),),
    paths(
        crate::credit::web::enquire_quote,
        crate::credit::web::lookup_quote,
        crate::credit::admin::list_pending_quotes,
        crate::credit::admin::list_accepted_quotes,
        crate::credit::admin::lookup_quote,
        crate::credit::admin::resolve_quote,
        crate::credit::web::resolve_offer,
    )
)]
struct ApiDoc;
