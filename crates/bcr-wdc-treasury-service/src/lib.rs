// ----- standard library imports
// ----- extra library imports
use axum::extract::FromRef;
use axum::routing::post;
use axum::Router;
use bitcoin::bip32 as btc32;
// ----- local modules
mod error;
mod persistence;
mod service;
mod web;
// ----- local imports

type ProdRepository = persistence::surreal::DBRepository;
type ProdService = service::Service<ProdRepository>;

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AppConfig {
    credit: persistence::surreal::ConnectionConfig,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    srvc: ProdService,
}

impl AppController {
    pub async fn new(seed: &[u8], cfg: AppConfig) -> Self {
        let repo = ProdRepository::new(cfg.credit)
            .await
            .expect("Failed to create repository");
        let xpriv = btc32::Xpriv::new_master(bitcoin::NetworkKind::Main, seed)
            .expect("Failed to create xpriv");
        let service = ProdService { repo, xpriv };
        Self { srvc: service }
    }
}

pub fn routes(app: AppController) -> Router {
    Router::new()
        .route(
            "/v1/credit/generate_blinds",
            post(web::generate_blind_messages),
        )
        .route("/v1/credit/store_signatures", post(web::store_signatures))
        .with_state(app)
}
