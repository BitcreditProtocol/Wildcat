// ----- standard library imports
// ----- extra library imports
use axum::extract::FromRef;
use axum::routing::{get, post};
use axum::Router;
use bitcoin::bip32 as btc32;
use bitcoin::secp256k1;
// ----- local modules
mod admin;
mod credit;
mod debit;
mod error;
mod persistence;
// ----- local imports

type ProdCreditRepository = persistence::surreal::CreditRepository;
type ProdCreditKeysService = credit::KeySrvc;
type ProdCreditService = credit::Service<ProdCreditRepository, ProdCreditKeysService>;

type ProdDebitWallet = debit::CDKWallet;
type ProdWildcatClient = debit::WildcatCl;
type ProdDebitRepository = persistence::surreal::DebitRepository;
type ProdDebitService = debit::Service<ProdDebitWallet, ProdWildcatClient, ProdDebitRepository>;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    credit_keys_service: credit::KeySrvcConfig,
    credit_repo: persistence::surreal::CreditConnectionConfig,
    debit_repo: persistence::surreal::DebitConnectionConfig,
    sat_wallet: debit::CDKWalletConfig,
    wildcat: debit::WildcatClientConfig,
    monitor_interval_sec: u64,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    crsat: ProdCreditService,
    sat: ProdDebitService,
}

impl AppController {
    pub async fn new(seed: &[u8], secret: secp256k1::SecretKey, cfg: AppConfig) -> Self {
        let AppConfig {
            credit_keys_service,
            credit_repo,
            debit_repo,
            sat_wallet,
            wildcat,
            monitor_interval_sec,
        } = cfg;
        let repo = ProdCreditRepository::new(credit_repo)
            .await
            .expect("Failed to create repository");
        let xpriv = btc32::Xpriv::new_master(bitcoin::NetworkKind::Main, seed)
            .expect("Failed to create xpriv");
        let keys = ProdCreditKeysService::new(credit_keys_service);
        let crsat = ProdCreditService { repo, xpriv, keys };

        let wallet = ProdDebitWallet::new(sat_wallet, seed)
            .await
            .expect("Failed to create wallet");
        let wdc = ProdWildcatClient::new(wildcat);
        let repo = ProdDebitRepository::new(debit_repo)
            .await
            .expect("Failed to create repository");
        let signing_keys =
            secp256k1::Keypair::from_secret_key(secp256k1::global::SECP256K1, &secret);
        let monitor_interval = tokio::time::Duration::from_secs(monitor_interval_sec);
        let sat = ProdDebitService {
            wallet,
            signing_keys,
            wdc,
            repo,
            monitor_interval,
        };
        sat.init_monitors_for_past_ebills()
            .await
            .expect("Failed to initialize monitors");

        Self { crsat, sat }
    }
}

pub fn routes(app: AppController) -> Router {
    Router::new().nest(
        "/v1/admin/treasury",
        Router::new()
            .route("/credit/generate_blinds", post(admin::generate_blinds))
            .route("/credit/store_signatures", post(admin::store_signatures))
            .route("/credit/balance", get(admin::crsat_balance))
            .route("/debit/redeem", post(admin::redeem))
            .route("/debit/balance", get(admin::sat_balance))
            .route(
                "/debit/request_to_mint_from_ebill",
                post(admin::request_mint_from_ebill),
            )
            .with_state(app),
    )
}
