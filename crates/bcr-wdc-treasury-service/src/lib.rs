// ----- standard library imports
use std::sync::Arc;
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
mod foreign;
mod persistence;
mod web;
// ----- local imports

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;

type ProdCreditRepository = persistence::surreal::CreditRepository;
type ProdCreditKeysService = credit::KeySrvc;
type ProdCreditService = credit::Service<ProdCreditRepository, ProdCreditKeysService>;
type ProdCrsatService = foreign::crsat::Service;
type ProdCrsatRepository = persistence::surreal::CrsatRepository;
type ProdCrsatKeysClient = foreign::clients::KeysCl;
type ProdCrsatClowderClient = foreign::clients::ClowderCl;

type ProdDebitWallet = debit::CDKWallet;
type ProdWildcatClient = debit::WildcatCl;
type ProdDebitRepository = persistence::surreal::DebitRepository;
type ProdDebitService = debit::Service<ProdDebitWallet, ProdWildcatClient, ProdDebitRepository>;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    credit_keys_url: reqwest::Url,
    credit_repo: persistence::surreal::CreditConnectionConfig,
    debit_repo: persistence::surreal::DebitConnectionConfig,
    crsat_repo: persistence::surreal::CrsatConnectionConfig,
    crsat_clowder_url: reqwest::Url,
    sat_wallet: debit::CDKWalletConfig,
    wildcat: debit::WildcatClientConfig,
    monitor_interval_sec: u64,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    credit: ProdCreditService,
    debit: ProdDebitService,
    crsat: ProdCrsatService,
}

impl AppController {
    pub async fn new(seed: &[u8], secret: secp256k1::SecretKey, cfg: AppConfig) -> Self {
        let AppConfig {
            credit_keys_url,
            crsat_clowder_url,
            credit_repo,
            debit_repo,
            crsat_repo,
            sat_wallet,
            wildcat,
            monitor_interval_sec,
        } = cfg;
        let repo = ProdCreditRepository::new(credit_repo)
            .await
            .expect("Failed to create repository");
        let xpriv = btc32::Xpriv::new_master(bitcoin::NetworkKind::Main, seed)
            .expect("Failed to create xpriv");
        let keys = ProdCreditKeysService::new(credit_keys_url.clone());
        let credit = ProdCreditService { repo, xpriv, keys };

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
        let debit = ProdDebitService {
            wallet,
            signing_keys,
            wdc,
            repo,
            monitor_interval,
        };
        debit
            .init_monitors_for_past_ebills()
            .await
            .expect("Failed to initialize monitors");

        let crsatrepo = ProdCrsatRepository::new(crsat_repo)
            .await
            .expect("Failed to create crsat repository");

        let crsatkeys = ProdCrsatKeysClient::new(credit_keys_url);
        let crsatclowder = ProdCrsatClowderClient::new(crsat_clowder_url);
        let crsat = ProdCrsatService {
            repo: Arc::new(crsatrepo),
            keys: Arc::new(crsatkeys),
            clowder: Arc::new(crsatclowder),
        };

        Self {
            credit,
            debit,
            crsat,
        }
    }
}

pub fn routes(app: AppController) -> Router {
    let web = Router::new()
        .route("/v1/treasury/redeem", post(web::redeem))
        .route("/v1/treasury/online_exchange", post(web::online_exchange));
    let admin = Router::new()
        .route(
            "/v1/admin/treasury/debit/request_to_mint_from_ebill",
            post(admin::request_mint_from_ebill),
        )
        .route(
            "/v1/admin/treasury/credit/generate_blinds",
            post(admin::generate_blinds),
        )
        .route(
            "/v1/admin/treasury/credit/store_signatures",
            post(admin::store_signatures),
        )
        .route(
            "/v1/admin/treasury/credit/balance",
            get(admin::crsat_balance),
        )
        .route(
            "/v1/admin/treasury/credit/try_htlc_swap",
            post(admin::try_htlc_swap),
        )
        .route("/v1/admin/treasury/debit/balance", get(admin::sat_balance));
    admin.merge(web).with_state(app)
}
