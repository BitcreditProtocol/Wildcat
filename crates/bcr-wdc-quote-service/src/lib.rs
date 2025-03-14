// ----- standard library imports
// ----- extra library imports
use axum::extract::FromRef;
use axum::routing::{get, post};
use axum::Router;
use cashu::nuts::nut00 as cdk00;
use cashu::nuts::nut12 as cdk12;
use utoipa::OpenApi;
// ----- local modules
mod admin;
mod error;
mod keys;
mod persistence;
mod quotes;
mod service;
mod utils;
mod web;
// ----- local imports

type TStamp = chrono::DateTime<chrono::Utc>;

pub type ProdQuoteRepository = persistence::surreal::DBQuotes;

pub type ProdKeysHandler = keys::KeysRestHandler;
pub type ProdQuotingService = service::Service<ProdKeysHandler, ProdQuoteRepository>;

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AppConfig {
    quotes: persistence::surreal::ConnectionConfig,
    keys: keys::KeysRestConfig,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    quote: ProdQuotingService,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig { quotes, keys } = cfg;
        let quotes_repository = ProdQuoteRepository::new(quotes)
            .await
            .expect("DB connection to quotes failed");

        let keys_hndlr = ProdKeysHandler::new(keys).expect("Keys handler creation failed");
        let quoting_service = ProdQuotingService {
            keys_hndlr,
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
        .route("/v1/mint/credit/quote", post(web::enquire_quote))
        .route("/v1/mint/credit/quote/:id", get(web::lookup_quote))
        .route("/v1/mint/credit/quote/:id", post(web::resolve_offer))
        .route(
            "/v1/admin/credit/quote/pending",
            get(admin::list_pending_quotes),
        )
        .route(
            "/v1/admin/credit/quote/accepted",
            get(admin::list_accepted_quotes),
        )
        .route("/v1/admin/credit/quote/:id", get(admin::admin_lookup_quote))
        .route(
            "/v1/admin/credit/quote/:id",
            post(admin::admin_resolve_quote),
        )
        .with_state(ctrl)
        .merge(swagger)
}

#[derive(utoipa::OpenApi)]
#[openapi(
    components(schemas(
        //bcr_ebill_core::contact::IdentityPublicData,
        bcr_wdc_webapi::quotes::BillInfo,
        bcr_wdc_webapi::quotes::IdentityPublicData,
        bcr_wdc_webapi::quotes::ContactType,
        bcr_wdc_webapi::quotes::PostalAddress,
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
        crate::admin::admin_lookup_quote,
        crate::admin::admin_resolve_quote,
        crate::web::resolve_offer,
    )
)]
struct ApiDoc;
