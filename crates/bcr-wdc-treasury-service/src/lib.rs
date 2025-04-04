// ----- standard library imports
// ----- extra library imports
use axum::extract::FromRef;
use axum::routing::post;
use axum::Router;
use bitcoin::bip32 as btc32;
use bitcoin::secp256k1;
// ----- local modules
mod credit;
mod debit;
mod error;
mod persistence;
mod web;
// ----- local imports

type ProdCrSatRepository = persistence::surreal::DBRepository;
type ProdCrSatService = credit::Service<ProdCrSatRepository>;

type ProdSatWallet = debit::CDKWallet;
type ProdSatService = debit::Service<ProdSatWallet>;

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AppConfig {
    crsat_repo: persistence::surreal::ConnectionConfig,
    cdk_mint_url: String,
    wallet_redb_storage: std::path::PathBuf,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    crsat: ProdCrSatService,
    sat: ProdSatService,
}

impl AppController {
    pub async fn new(seed: &[u8], secret: secp256k1::SecretKey, cfg: AppConfig) -> Self {
        let repo = ProdCrSatRepository::new(cfg.crsat_repo)
            .await
            .expect("Failed to create repository");
        let xpriv = btc32::Xpriv::new_master(bitcoin::NetworkKind::Main, seed)
            .expect("Failed to create xpriv");
        let crsat = ProdCrSatService { repo, xpriv };

        let wallet = ProdSatWallet::new(&cfg.cdk_mint_url, &cfg.wallet_redb_storage, seed)
            .await
            .expect("Failed to create wallet");
        let secp_ctx = secp256k1::Secp256k1::signing_only();
        let signing_keys = secp256k1::Keypair::from_secret_key(&secp_ctx, &secret);
        let sat = ProdSatService {
            wallet,
            secp_ctx,
            signing_keys,
        };

        Self { crsat, sat }
    }
}

pub fn routes(app: AppController) -> Router {
    Router::new()
        .route(
            "/v1/credit/generate_blinds",
            post(web::generate_blind_messages),
        )
        .route("/v1/credit/store_signatures", post(web::store_signatures))
        .route(
            "/v1/debit/request_to_mint_from_ebill",
            post(web::request_mint_from_ebill),
        )
        .with_state(app)
}
