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
use clwdr_client::{ClowderNatsClient, ClowderRestClient, SignatoryNatsClient};
// ----- local modules
mod admin;
mod debit;
mod devmode;
mod error;
mod foreign;
mod persistence;
mod web;
// ----- local imports

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;

type ProdCrsatService = foreign::crsat::Service;
type ProdCrsatOnlineRepository = persistence::surreal::ForeignOnlineRepository;
type ProdCrsatOfflineRepository = persistence::surreal::ForeignOfflineRepository;
type ProdCrsatKeysClient = foreign::clients::CrsatKeysClient;
type ProdClowderClient = foreign::clients::ClowderCl;
type ProdSatService = foreign::sat::Service;
type ProdSatOnlineRepository = persistence::surreal::ForeignOnlineRepository;
type ProdSatOfflineRepository = persistence::surreal::ForeignOfflineRepository;
type ProdSatKeysClient = foreign::clients::SatKeysClient;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    credit_keys_url: reqwest::Url,
    cdk_mintd_url: cashu::MintUrl,
    debit_repo: persistence::surreal::DebitConnectionConfig,
    crsatonline_repo: persistence::surreal::ForeignOnlineConnectionConfig,
    crsatoffline_repo: persistence::surreal::ForeignOfflineConnectionConfig,
    satonline_repo: persistence::surreal::ForeignOnlineConnectionConfig,
    satoffline_repo: persistence::surreal::ForeignOfflineConnectionConfig,
    clowder_url: reqwest::Url,
    clwdr_nats_url: Option<reqwest::Url>,
    signer_url: reqwest::Url,
    sat_wallet: debit::CDKWalletConfig,
    wildcat: debit::WildcatClientConfig,
    monitor_interval_sec: u64,
    quote_expiry_seconds: u64,
    min_confirmations: u32,
    min_melt_threshold: bitcoin::Amount,
    min_mint_threshold: bitcoin::Amount,
}

#[derive(Clone)]
struct Parameters {
    pub min_confirmations: u32,
    pub min_melt_threshold: bitcoin::Amount,
    pub min_mint_threshold: bitcoin::Amount,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    debit: debit::Service,
    crsat: Arc<ProdCrsatService>,
    sat: Arc<ProdSatService>,
    signer: Arc<SignatoryNatsClient>,
    clwdr_nats: Option<Arc<ClowderNatsClient>>,
    clwdr_rest: Arc<ClowderRestClient>,
    dbmint: cdk::wallet::HttpClient,
    dev: Arc<devmode::Service>,
    params: Parameters,
}

impl AppController {
    pub async fn new(seed: [u8; 64], secret: secp256k1::SecretKey, cfg: AppConfig) -> Self {
        let AppConfig {
            credit_keys_url,
            cdk_mintd_url,
            clowder_url,
            clwdr_nats_url,
            signer_url,
            debit_repo,
            crsatonline_repo,
            crsatoffline_repo,
            satonline_repo,
            satoffline_repo,
            sat_wallet,
            wildcat,
            monitor_interval_sec,
            quote_expiry_seconds,
            min_confirmations,
            min_melt_threshold,
            min_mint_threshold,
        } = cfg;

        let wallet = debit::CDKWallet::new(sat_wallet, seed)
            .await
            .expect("Failed to create wallet");
        let wdc = debit::WildcatCl::new(wildcat);
        let repo = persistence::surreal::DebitRepository::new(debit_repo)
            .await
            .expect("Failed to create repository");
        let signing_keys =
            secp256k1::Keypair::from_secret_key(secp256k1::global::SECP256K1, &secret);
        tracing::info!("signing public key: {}", signing_keys.public_key());
        let monitor_interval = tokio::time::Duration::from_secs(monitor_interval_sec);
        let clowder_cl = debit::ClowderCl(ClowderRestClient::new(clowder_url.clone()));
        let clwdr_nats = if let Some(url) = clwdr_nats_url {
            Some(Arc::new(
                ClowderNatsClient::new(url)
                    .await
                    .expect("Failed to create clowder nats client"),
            ))
        } else {
            None
        };
        let clowder_write: Option<Arc<dyn debit::ClowderWriteService>> = clwdr_nats
            .as_ref()
            .map(|c| Arc::new(debit::ClowderNatsCl(c.clone())) as Arc<dyn debit::ClowderWriteService>);
        let debit = debit::Service {
            wallet: Arc::new(wallet),
            signing_keys,
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_read: Arc::new(clowder_cl),
            clowder_write,
            monitor_interval,
            quote_expiry_seconds,
        };
        debit
            .init_monitors_for_past_ebills()
            .await
            .expect("Failed to initialize monitors");

        let crsatonlinerepo = Arc::new(
            ProdCrsatOnlineRepository::new(crsatonline_repo)
                .await
                .expect("Failed to create crsat online repository"),
        );
        let crsatofflinerepo = Arc::new(
            ProdCrsatOfflineRepository::new(crsatoffline_repo)
                .await
                .expect("Failed to create crsat offline repository"),
        );
        let crsatkeys = ProdCrsatKeysClient::new(credit_keys_url.clone());
        let clwdr_rest = Arc::new(ClowderRestClient::new(clowder_url.clone()));
        let clowder = Arc::new(ProdClowderClient::new(clowder_url));
        let factory = Arc::new(foreign::clients::MintClientFactory {});
        let interval = std::time::Duration::from_secs(monitor_interval_sec);
        let settler = {
            let online: Arc<dyn foreign::OnlineRepository> = crsatonlinerepo.clone();
            let offline: Arc<dyn foreign::OfflineRepository> = crsatofflinerepo.clone();
            let clwdr: Arc<dyn foreign::ClowderClient> = clowder.clone();
            let fctry: Arc<dyn foreign::MintClientFactory> = factory.clone();
            Box::new(foreign::settle::Handler::new(
                &online, &offline, &clwdr, &fctry, interval,
            ))
        };
        let crsat = Arc::new(ProdCrsatService {
            online_repo: crsatonlinerepo,
            offline_repo: crsatofflinerepo,
            keys: Box::new(crsatkeys),
            clowder: clowder.clone(),
            mint_factory: factory.clone(),
            settler,
        });

        let satonlinerepo = Arc::new(
            ProdSatOnlineRepository::new(satonline_repo)
                .await
                .expect("Failed to create sat repository"),
        );
        let satofflinerepo = Arc::new(
            ProdSatOfflineRepository::new(satoffline_repo)
                .await
                .expect("Failed to create sat offline repository"),
        );
        let satkeys = ProdSatKeysClient::new(cdk_mintd_url.clone(), signing_keys);
        let settler = {
            let online: Arc<dyn foreign::OnlineRepository> = satonlinerepo.clone();
            let offline: Arc<dyn foreign::OfflineRepository> = satofflinerepo.clone();
            let clwdr: Arc<dyn foreign::ClowderClient> = clowder.clone();
            let fctry: Arc<dyn foreign::MintClientFactory> = factory.clone();
            Box::new(foreign::settle::Handler::new(
                &online, &offline, &clwdr, &fctry, interval,
            ))
        };
        let sat = Arc::new(ProdSatService {
            keys: Box::new(satkeys),
            online_repo: satonlinerepo,
            offline_repo: satofflinerepo,
            clowder,
            mint_factory: factory,
            settler,
        });

        let signer = SignatoryNatsClient::new(signer_url, None)
            .await
            .expect("Failed to create signer");

        let dbmint = cdk::wallet::HttpClient::new(cdk_mintd_url.clone());
        let dev = devmode::Service {
            crkeys: bcr_common::client::keys::Client::new(credit_keys_url),
            dbmint: dbmint.clone(),
        };
        Self {
            debit,
            crsat,
            sat,
            signer: Arc::new(signer),
            clwdr_nats,
            clwdr_rest,
            dbmint,
            dev: Arc::new(dev),
            params: Parameters {
                min_confirmations,
                min_melt_threshold,
                min_mint_threshold,
            },
        }
    }

    pub async fn stop(&self) -> error::Result<()> {
        self.crsat.stop().await?;
        self.sat.stop().await
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
        )
        .route(
            TreasuryClient::CRSATEXCHANGEOFFLINE_EP_V1,
            post(web::crsat_offline_exchange),
        )
        .route("/v1/free_money", post(devmode::free_money))
        .route(
            TreasuryClient::SATEXCHANGEOFFLINE_EP_V1,
            post(web::sat_offline_exchange),
        )
        .route(
            TreasuryClient::MELTQUOTE_ONCHAIN_EP_V1,
            post(web::melt_quote_onchain),
        )
        .route(TreasuryClient::MELT_ONCHAIN_EP_V1, post(web::melt_onchain))
        .route(
            TreasuryClient::MINTQUOTE_ONCHAIN_EP_V1,
            post(web::mint_quote_onchain),
        )
        .route(
            TreasuryClient::MINTQUOTE_ONCHAIN_GET_EP_V1,
            get(web::get_mint_quote_onchain),
        )
        .route(TreasuryClient::MINT_ONCHAIN_EP_V1, post(web::mint_onchain));
    let admin = Router::new()
        .route(
            TreasuryClient::REQTOPAY_EP_V1,
            post(admin::request_to_pay_ebill),
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
