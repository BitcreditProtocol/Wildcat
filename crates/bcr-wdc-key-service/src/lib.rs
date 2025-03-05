// ----- standard library imports
// ----- extra library imports
use axum::extract::FromRef;
use axum::routing::get;
use axum::Router;
use utoipa::OpenApi;
// ----- local modules
mod error;
mod persistence;
mod service;
mod web;
// ----- local imports

pub type ProdKeysRepository = persistence::surreal::DB;
pub type ProdKeysService = service::Service<ProdKeysRepository>;

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AppConfig {
    keys_cfg: persistence::surreal::ConnectionConfig,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    keys: ProdKeysService,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let repo = ProdKeysRepository::new(cfg.keys_cfg)
            .await
            .expect("DB connection to keys failed");
        let srv = ProdKeysService { keys: repo };
        Self { keys: srv }
    }
}
pub fn routes(ctrl: AppController) -> Router {
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    Router::new()
        .route("/v1/keysets/:kid", get(web::lookup_keyset))
        .route("/v1/keys/:kid", get(web::lookup_keys))
        .with_state(ctrl)
        .merge(swagger)
}

#[derive(utoipa::OpenApi)]
#[openapi(components(schemas(),), paths())]
struct ApiDoc;
