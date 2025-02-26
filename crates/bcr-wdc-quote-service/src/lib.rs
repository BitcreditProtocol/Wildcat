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
mod admin;
mod error;
mod keys_factory;
mod persistence;
mod quotes;
mod utils;
mod web;
// ----- local imports

type TStamp = chrono::DateTime<chrono::Utc>;

pub type ProdQuoteKeysRepository = persistence::surreal::keysets::QuoteKeysDB;
pub type ProdKeysRepository = persistence::surreal::keysets::KeysDB;
pub type ProdActiveKeysRepository = persistence::surreal::keysets::KeysDB;
pub type ProdQuoteRepository = persistence::surreal::quotes::DB;

pub type ProdCreditKeysFactory = keys_factory::Factory<ProdQuoteKeysRepository, ProdKeysRepository>;
pub type ProdQuotingService = quotes::Service<ProdCreditKeysFactory, ProdQuoteRepository>;

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AppConfig {
    dbs: persistence::surreal::DBConfig,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    quote: ProdQuotingService,
}

impl AppController {
    pub async fn new(mint_seed: &[u8], cfg: AppConfig) -> Self {
        let AppConfig { dbs, .. } = cfg;
        let persistence::surreal::DBConfig {
            quotes,
            quotes_keys,
            maturity_keys,
            ..
        } = dbs;
        let quotes_repository = ProdQuoteRepository::new(quotes)
            .await
            .expect("DB connection to quotes failed");
        let quote_keys_repository = ProdQuoteKeysRepository::new(quotes_keys)
            .await
            .expect("DB connection to quoteskeys failed");
        let maturity_keys_repository = ProdKeysRepository::new(maturity_keys)
            .await
            .expect("DB connection to maturity_keys failed");

        let keys_factory = ProdCreditKeysFactory::new(
            mint_seed,
            quote_keys_repository,
            maturity_keys_repository.clone(),
        );
        let quoting_service = ProdQuotingService {
            keys_gen: keys_factory,
            quotes: quotes_repository,
        };

        Self {
            quote: quoting_service,
        }
    }
}
pub fn credit_routes(ctrl: AppController) -> Router {
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    Router::new()
        .route("/v1/credit/mint/quote", post(web::enquire_quote))
        .route("/v1/credit/mint/quote/:id", get(web::lookup_quote))
        .route(
            "/v1/credit/mint/quote/:id",
            post(web::resolve_offer),
        )
        .route(
            "/v1/admin/credit/quote/pending",
            get(admin::list_pending_quotes),
        )
        .route(
            "/v1/admin/credit/quote/accepted",
            get(admin::list_accepted_quotes),
        )
        .route(
            "/v1/admin/credit/quote/:id",
            get(admin::lookup_quote),
        )
        .route(
            "/v1/admin/credit/quote/:id",
            post(admin::resolve_quote),
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
        crate::web::enquire_quote,
        crate::web::lookup_quote,
        crate::admin::list_pending_quotes,
        crate::admin::list_accepted_quotes,
        crate::admin::lookup_quote,
        crate::admin::resolve_quote,
        crate::web::resolve_offer,
    )
)]
struct ApiDoc;
