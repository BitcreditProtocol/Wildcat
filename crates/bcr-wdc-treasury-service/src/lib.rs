// ----- standard library imports
// ----- extra library imports
use axum::extract::FromRef;
use axum::routing::{get, post};
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
type ProdCrSatKeysService = credit::KeySrvc;
type ProdCrSatService = credit::Service<ProdCrSatRepository, ProdCrSatKeysService>;

type ProdSatWallet = debit::CDKWallet;
type ProdProofClient = debit::ProofCl;
type ProdSatService = debit::Service<ProdSatWallet, ProdProofClient>;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    crsat_keys_service: credit::KeySrvcConfig,
    crsat_repo: persistence::surreal::ConnectionConfig,
    sat_wallet: debit::CDKWalletConfig,
    proof_client: debit::ProofClientConfig,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    crsat: ProdCrSatService,
    sat: ProdSatService,
}

impl AppController {
    pub async fn new(seed: &[u8], secret: secp256k1::SecretKey, cfg: AppConfig) -> Self {
        let AppConfig {
            crsat_keys_service,
            crsat_repo,
            sat_wallet,
            proof_client,
        } = cfg;
        let repo = ProdCrSatRepository::new(crsat_repo)
            .await
            .expect("Failed to create repository");
        let xpriv = btc32::Xpriv::new_master(bitcoin::NetworkKind::Main, seed)
            .expect("Failed to create xpriv");
        let keys = ProdCrSatKeysService::new(crsat_keys_service);
        let crsat = ProdCrSatService { repo, xpriv, keys };

        let wallet = ProdSatWallet::new(sat_wallet, seed)
            .await
            .expect("Failed to create wallet");
        let proof_client = ProdProofClient::new(proof_client);
        let signing_keys =
            secp256k1::Keypair::from_secret_key(bitcoin::secp256k1::global::SECP256K1, &secret);
        let sat = ProdSatService {
            wallet,
            signing_keys,
            proof: proof_client,
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
        .route("/v1/debit/redeem", post(web::redeem))
        .route("/v1/balance/credit", get(web::crsat_balance))
        .route("/v1/balance/debit", get(web::sat_balance))
        .with_state(app)
}
