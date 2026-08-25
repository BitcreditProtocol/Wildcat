// ----- standard library imports
use std::{str::FromStr, sync::Arc};
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{delete, get, patch, post, put},
    Router,
};
use bcr_common::{
    cashu,
    client::{
        admin::clowder::Client as ClowderClient, core::Client as CoreClient,
        ebill::Client as EBillClient, quote, treasury::Client as TreasuryClient, Url as ClientUrl,
    },
    wire::clowder as wire_clowder,
};
use bcr_wdc_utils::{routine::RoutineHandle, surreal};
// ----- local modules
mod admin;
mod authorization;
mod client;
mod credit_evidence;
mod error;
mod monitor;
mod persistence;
mod quotes;
mod service;
mod web;
// ----- local imports

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;

pub const MINIMUM_MONITOR_INTERVAL_SECONDS: u64 = 5;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    quotes: surreal::DBConnConfig,
    core_url: ClientUrl,
    treasury_url: ClientUrl,
    ebill_url: ClientUrl,
    clowder_url: reqwest::Url,
    monitor_interval_seconds: u64,
    credit_program_version: String,
    credit_program_digest: String,
    credit_authorization_mint_id: String,
    credit_authorization_key_id: String,
    credit_authorization_public_key: String,
    credit_evidence_risk_methodology_version: String,
    credit_evidence_risk_assessed_by: String,
    credit_evidence_capacity_methodology_version: String,
    credit_evidence_capacity_assessed_by: String,
    credit_evidence_risk_authority_key_id: String,
    credit_evidence_risk_authority_public_key: String,
    credit_evidence_capacity_authority_key_id: String,
    credit_evidence_capacity_authority_public_key: String,
    credit_evidence_allow_synthetic: bool,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    quote: Arc<service::Service>,
    credit_evidence: Arc<credit_evidence::Store>,
}

pub async fn init_app(cfg: AppConfig) -> (AppController, RoutineHandle) {
    let AppConfig {
        quotes,
        core_url,
        treasury_url,
        ebill_url,
        clowder_url,
        monitor_interval_seconds,
        credit_program_version,
        credit_program_digest,
        credit_authorization_mint_id,
        credit_authorization_key_id,
        credit_authorization_public_key,
        credit_evidence_risk_methodology_version,
        credit_evidence_risk_assessed_by,
        credit_evidence_capacity_methodology_version,
        credit_evidence_capacity_assessed_by,
        credit_evidence_risk_authority_key_id,
        credit_evidence_risk_authority_public_key,
        credit_evidence_capacity_authority_key_id,
        credit_evidence_capacity_authority_public_key,
        credit_evidence_allow_synthetic,
    } = cfg;
    let credit_program =
        quotes::CreditProgramBinding::new(credit_program_version, credit_program_digest)
            .expect("invalid Mint credit program binding");
    let credit_evidence_mint_id = credit_authorization_mint_id.clone();
    let authorization_verifier = authorization::AuthorizationVerifier::new(
        credit_authorization_mint_id,
        credit_authorization_key_id,
        credit_authorization_public_key,
    )
    .expect("invalid AI Credit authorization verifier configuration");
    let credit_evidence = Arc::new(
        credit_evidence::Store::new(
            quotes.clone(),
            credit_evidence::Settings {
                mint_id: credit_evidence_mint_id,
                risk_methodology_version: credit_evidence_risk_methodology_version,
                risk_assessed_by: credit_evidence_risk_assessed_by,
                capacity_methodology_version: credit_evidence_capacity_methodology_version,
                capacity_assessed_by: credit_evidence_capacity_assessed_by,
                risk_authority_key_id: credit_evidence_risk_authority_key_id,
                risk_authority_public_key: credit_evidence_risk_authority_public_key,
                capacity_authority_key_id: credit_evidence_capacity_authority_key_id,
                capacity_authority_public_key: credit_evidence_capacity_authority_public_key,
                allow_synthetic: credit_evidence_allow_synthetic,
            }
            .validate()
            .expect("invalid Mint credit evidence configuration"),
        )
        .await
        .expect("DB connection to Mint credit evidence failed"),
    );
    let quotes_repository = persistence::surreal::DBQuotes::new(quotes)
        .await
        .expect("DB connection to quotes failed");

    let clwdr_cl = ClowderClient::new(clowder_url);
    let public_key = clwdr_cl
        .get_info()
        .await
        .expect("Failed to get Clowder ID")
        .node_id;
    let wire_clowder::MintUrlResponse { mint_url, .. } = clwdr_cl
        .get_mint_url(&public_key)
        .await
        .expect("Failed to get mint URL");
    let core = CoreClient::new(core_url);
    let treasury_cl = TreasuryClient::new(treasury_url);
    let ebill = EBillClient::new(ebill_url);
    let wdc_cl = client::WildcatCl {
        core,
        treasury: treasury_cl,
        ebill,
    };
    let cashu_mint_url =
        cashu::MintUrl::from_str(mint_url.as_ref()).expect("cashu::MintUrl == reqwest::Url");
    let quoting_service = service::Service {
        wdc_client: Box::new(wdc_cl),
        quotes: Box::new(quotes_repository),
        mint_url: cashu_mint_url,
        credit_program,
        authorization_verifier,
        credit_evidence: Some(credit_evidence.clone()),
    };
    let quote = Arc::new(quoting_service);
    let monitor = monitor::EbillMonitor {
        srvc: quote.clone(),
    };
    let interval = std::time::Duration::from_secs(std::cmp::max(
        monitor_interval_seconds,
        MINIMUM_MONITOR_INTERVAL_SECONDS,
    ));
    let routine_handle = RoutineHandle::new(monitor, interval);
    (
        AppController {
            quote,
            credit_evidence,
        },
        routine_handle,
    )
}

pub fn routes(ctrl: AppController) -> Router {
    let web = Router::new()
        .route("/health", get(get_health))
        .route(quote::web_ep::ENQUIRE_V1, post(web::enquire_quote))
        .route(
            quote::web_ep::REISSUE_ENQUIRE_V1,
            post(web::reissue_enquire_quote),
        )
        .route(quote::web_ep::LOOKUP_V1, get(web::lookup_quote))
        .route(quote::web_ep::RESOLVE_V1, delete(web::cancel))
        .route(quote::web_ep::RESOLVE_V1, patch(web::resolve_offer));

    let admin = Router::new()
        .route(quote::admin_ep::LIST, get(admin::list_quotes))
        .route(quote::admin_ep::LOOKUP, get(admin::lookup_quote))
        .route(quote::admin_ep::UPDATE, patch(admin::update_quote))
        .route(quote::admin_ep::AUTHORIZE, patch(admin::authorize_quote))
        .route(
            quote::admin_ep::ACCEPTOR_RISK_EVIDENCE,
            put(admin::record_acceptor_risk_evidence),
        )
        .route(
            quote::admin_ep::MINT_CAPACITY_EVIDENCE,
            put(admin::record_mint_capacity_evidence),
        )
        .route(
            quote::admin_ep::ENABLE_MINTING,
            patch(admin::enable_minting),
        )
        .route(
            quote::admin_ep::SHARED_EBILL_HISTORY,
            get(admin::get_shared_ebill_history),
        );

    Router::new().merge(web).merge(admin).with_state(ctrl)
}

async fn get_health() -> &'static str {
    "{ \"status\": \"OK\" }"
}
