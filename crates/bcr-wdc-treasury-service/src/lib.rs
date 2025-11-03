// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{get, post},
    Router,
};
use bcr_wdc_treasury_client::TreasuryClient;
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
type ProdCrsatRepository = persistence::surreal::ForeignRepository;
type ProdCrsatKeysClient = foreign::clients::CrsatKeysClient;
type ProdClowderClient = foreign::clients::ClowderCl;
type ProdSatService = foreign::sat::Service;
type ProdSatRepository = persistence::surreal::ForeignRepository;
type ProdSatKeysClient = foreign::clients::SatKeysClient;

type ProdDebitWallet = debit::CDKWallet;
type ProdWildcatClient = debit::WildcatCl;
type ProdDebitRepository = persistence::surreal::DebitRepository;
type ProdDebitService = debit::Service<ProdDebitWallet, ProdWildcatClient, ProdDebitRepository>;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    credit_keys_url: reqwest::Url,
    cdk_mintd_url: cashu::MintUrl,
    credit_repo: persistence::surreal::CreditConnectionConfig,
    debit_repo: persistence::surreal::DebitConnectionConfig,
    crsat_repo: persistence::surreal::ForeignConnectionConfig,
    sat_repo: persistence::surreal::ForeignConnectionConfig,
    clowder_url: reqwest::Url,
    sat_wallet: debit::CDKWalletConfig,
    wildcat: debit::WildcatClientConfig,
    monitor_interval_sec: u64,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    credit: ProdCreditService,
    debit: ProdDebitService,
    crsat: ProdCrsatService,
    sat: ProdSatService,
}

impl AppController {
    pub async fn new(seed: [u8; 64], secret: secp256k1::SecretKey, cfg: AppConfig) -> Self {
        let AppConfig {
            credit_keys_url,
            cdk_mintd_url,
            clowder_url,
            credit_repo,
            debit_repo,
            crsat_repo,
            sat_repo,
            sat_wallet,
            wildcat,
            monitor_interval_sec,
        } = cfg;
        let repo = ProdCreditRepository::new(credit_repo)
            .await
            .expect("Failed to create repository");
        let keys = ProdCreditKeysService::new(credit_keys_url.clone());
        let credit = ProdCreditService { repo, keys };

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
        let clowder = Arc::new(ProdClowderClient::new(clowder_url));
        let crsat = ProdCrsatService {
            repo: Arc::new(crsatrepo),
            keys: Arc::new(crsatkeys),
            clowder: clowder.clone(),
        };

        let satrepo = ProdSatRepository::new(sat_repo)
            .await
            .expect("Failed to create sat repository");
        let satkeys = ProdSatKeysClient::new(cdk_mintd_url, signing_keys);
        let sat = ProdSatService {
            keys: Arc::new(satkeys),
            repo: Arc::new(satrepo),
            clowder,
        };

        Self {
            credit,
            debit,
            crsat,
            sat,
        }
    }
}

pub fn routes(app: AppController) -> Router {
    let web = Router::new()
        .route(TreasuryClient::REDEEM_EP_V1, post(web::redeem))
        .route(
            TreasuryClient::CRSATEXCHANGEONLINE_EP_V1,
            post(web::crsat_online_exchange),
        )
        .route(
            TreasuryClient::SATEXCHANGEONLINE_EP_V1,
            post(web::sat_online_exchange),
        );
    let admin = Router::new()
        .route(
            "/v1/admin/treasury/debit/request_to_mint_from_ebill",
            post(admin::request_mint_from_ebill),
        )
        .route(
            TreasuryClient::GENERATEBLINDS_EP_V1,
            post(admin::generate_blinds),
        )
        .route(
            TreasuryClient::STORESIGNATURES_EP_V1,
            post(admin::store_signatures),
        )
        .route(
            TreasuryClient::CRSATBALANCE_EP_V1,
            get(admin::crsat_balance),
        )
        .route(
            TreasuryClient::TRYCRSATHTLC_EP_V1,
            post(admin::crsat_try_htlc_swap),
        )
        .route(
            TreasuryClient::TRYSATHTLC_EP_V1,
            post(admin::sat_try_htlc_swap),
        )
        .route(TreasuryClient::SATBALANCE_EP_V1, get(admin::sat_balance));
    admin.merge(web).with_state(app)
}
