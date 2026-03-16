// ----- standard library imports
use std::sync::{Arc, Mutex};
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{get, post},
    Router,
};
use bcr_common::{
    cashu, cdk,
    client::{core::Client as CoreClient, treasury::Client as TreasuryClient, Url as ClientUrl},
};
use bcr_wdc_utils::surreal;
use bitcoin::secp256k1;
use clwdr_client::{ClowderNatsClient, ClowderRestClient, SignatoryNatsClient};
// ----- local modules
mod admin;
mod credit;
mod debit;
mod devmode;
mod error;
mod foreign;
mod persistence;
mod web;
// ----- local imports

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;

type ProdCrsatOnlineRepository = persistence::surreal::ForeignOnlineRepository;
type ProdCrsatOfflineRepository = persistence::surreal::ForeignOfflineRepository;
type ProdSatService = foreign::sat::Service;
type ProdSatOnlineRepository = persistence::surreal::ForeignOnlineRepository;
type ProdSatOfflineRepository = persistence::surreal::ForeignOfflineRepository;
type ProdSatKeysClient = foreign::clients::SatKeysClient;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    debit: DebitConfig,
    foreign: ForeignConfig,
    credit: CreditConfig,
    core_url: ClientUrl,
    clowder_rest_url: reqwest::Url,
    clowder_nats_url: Option<reqwest::Url>,
    signer_url: reqwest::Url,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct DebitConfig {
    cdk_mintd_url: cashu::MintUrl,
    db: surreal::DBConnConfig,
    sat_wallet: debit::CDKWalletConfig,
    wildcat: debit::WildcatClientConfig,
    monitor_interval_sec: u32,
    quote_expiry_seconds: u32,
    min_confirmations: u32,
    min_melt_threshold: bitcoin::Amount,
    min_mint_threshold: bitcoin::Amount,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct ForeignConfig {
    crsatonline_repo: surreal::DBConnConfig,
    crsatoffline_repo: surreal::DBConnConfig,
    satonline_repo: surreal::DBConnConfig,
    satoffline_repo: surreal::DBConnConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct CreditConfig {
    db: surreal::DBConnConfig,
}

#[derive(Clone)]
struct Parameters {
    pub min_melt_threshold: bitcoin::Amount,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    credit: Arc<credit::Service>,
    debit: debit::Service,
    crsat: Arc<foreign::crsat::Service>,
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
            debit,
            foreign,
            credit,
            core_url,
            clowder_rest_url,
            clowder_nats_url,
            signer_url,
        } = cfg;
        let DebitConfig {
            cdk_mintd_url,
            db: debit_repo,
            sat_wallet,
            wildcat,
            monitor_interval_sec,
            quote_expiry_seconds,
            min_confirmations,
            min_melt_threshold,
            min_mint_threshold,
        } = debit;
        let ForeignConfig {
            crsatonline_repo,
            crsatoffline_repo,
            satonline_repo,
            satoffline_repo,
        } = foreign;
        let CreditConfig { db: mintops } = credit;

        //clients
        let core_client = Arc::new(CoreClient::new(core_url));
        let clowder_rest_client = Arc::new(ClowderRestClient::new(clowder_rest_url));
        let clowder_nats_client = if let Some(url) = clowder_nats_url {
            Some(Arc::new(
                ClowderNatsClient::new(url)
                    .await
                    .expect("Failed to create clowder nats client"),
            ))
        } else {
            None
        };
        let signer_client = Arc::new(
            SignatoryNatsClient::new(signer_url, None)
                .await
                .expect("Failed to create signer"),
        );

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
        let monitor_interval = chrono::Duration::seconds(monitor_interval_sec as i64);
        let clowder_cl = debit::ClowderCl {
            rest: clowder_rest_client.clone(),
            nats: clowder_nats_client.clone(),
            signatory: signer_client.clone(),
            min_confirmations,
        };
        let dbmint = cdk::wallet::HttpClient::new(cdk_mintd_url.clone());
        let debit = debit::Service {
            wallet: Arc::new(wallet),
            signing_keys,
            monitor_interval,
            quote_expiry: chrono::Duration::seconds(quote_expiry_seconds as i64),
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder_cl),
            cancel: tokio_util::sync::CancellationToken::new(),
            hndls: Arc::new(Mutex::new(Vec::new())),
            dbmint: dbmint.clone(),
            min_mint_threshold,
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
        let crsatcore = foreign::clients::CrsatCoreClient {
            core: core_client.clone(),
        };
        let clowder = Arc::new(foreign::clients::ClowderCl {
            clwdr: clowder_rest_client.clone(),
        });
        let factory = Arc::new(foreign::clients::MintClientFactory {});
        let interval = std::time::Duration::from_secs(monitor_interval_sec as u64);
        let settler = {
            let online: Arc<dyn foreign::OnlineRepository> = crsatonlinerepo.clone();
            let offline: Arc<dyn foreign::OfflineRepository> = crsatofflinerepo.clone();
            let clwdr: Arc<dyn foreign::ClowderClient> = clowder.clone();
            let fctry: Arc<dyn foreign::MintClientFactory> = factory.clone();
            Box::new(foreign::settle::Handler::new(
                &online, &offline, &clwdr, &fctry, interval,
            ))
        };
        let crsat = Arc::new(foreign::crsat::Service {
            online_repo: crsatonlinerepo,
            offline_repo: crsatofflinerepo,
            keys: Box::new(crsatcore),
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

        let dev = devmode::Service {
            crcore: core_client.clone(),
            dbmint: dbmint.clone(),
        };
        let credit_repo = persistence::surreal::DBCredit::new(mintops)
            .await
            .expect("Failed to create mintops repository");
        let corecl = credit::CoreCl(core_client.clone());
        let clowdercl = credit::new_clowder_client(clowder_nats_client.clone());
        let credit = Arc::new(credit::Service {
            repo: Box::new(credit_repo),
            corecl: Box::new(corecl),
            clowdercl,
        });
        Self {
            credit,
            debit,
            crsat,
            sat,
            signer: signer_client.clone(),
            clwdr_rest: clowder_rest_client.clone(),
            clwdr_nats: clowder_nats_client.clone(),
            dbmint,
            dev: Arc::new(dev),
            params: Parameters { min_melt_threshold },
        }
    }

    pub async fn stop(&self) -> error::Result<()> {
        self.debit.stop().await?;
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
        .route(TreasuryClient::MINT_EP_V1, post(web::mint_ebill));
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
        .route(TreasuryClient::SATBALANCE_EP_V1, get(admin::sat_balance))
        .route(
            TreasuryClient::IS_EBILL_MINT_COMPLETE_EP_V1,
            get(admin::is_ebill_minting_completed),
        )
        .route(TreasuryClient::NEWMINTOP_EP_V1, post(admin::new_mintop))
        .route(TreasuryClient::LISTMINTOPS_EP_V1, get(admin::list_mintops))
        .route(
            TreasuryClient::MINTOPSTATUS_EP_V1,
            get(admin::mintop_status),
        );
    admin.merge(web).with_state(app)
}
