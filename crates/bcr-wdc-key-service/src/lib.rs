// ----- standard library imports
// ----- extra library imports
use axum::extract::FromRef;
use axum::routing::{get, post};
use axum::Router;
use cashu::nut00 as cdk00;
use cashu::nut02 as cdk02;
use utoipa::OpenApi;
// ----- local modules
mod admin;
mod error;
mod persistence;
mod service;
mod web;
// ----- local imports

pub type ProdKeysRepository = persistence::surreal::DB;
pub type ProdKeysService = service::Service<ProdKeysRepository>;

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AppConfig {
    keys: persistence::surreal::ConnectionConfig,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    keys: ProdKeysService,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let repo = ProdKeysRepository::new(cfg.keys)
            .await
            .expect("DB connection to keys failed");
        let srv = ProdKeysService { keys: repo };
        Self { keys: srv }
    }
}
pub fn routes(ctrl: AppController) -> Router {
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    let web = Router::new()
        .route("/v1/keysets/:kid", get(web::lookup_keyset))
        .route("/v1/keys/:kid", get(web::lookup_keys));
    // separate admin as it will likely have different auth requirements
    let admin = Router::new()
        .route("/v1/admin/keys/sign", post(admin::sign_blind))
        .route("/v1/admin/keys/verify", post(admin::verify_proof));

    Router::new()
        .merge(web)
        .merge(admin)
        .with_state(ctrl)
        .merge(swagger)
}

#[derive(utoipa::OpenApi)]
#[openapi(
    components(schemas(
        cdk00::BlindedMessage,
        cdk00::BlindSignature,
        cdk00::Proof,
        cdk02::Id,
        cdk02::KeySetInfo,
        cdk02::KeySet,
    ),),
    paths(
        web::lookup_keyset,
        web::lookup_keys,
        admin::sign_blind,
        admin::verify_proof,
    )
)]
struct ApiDoc;
