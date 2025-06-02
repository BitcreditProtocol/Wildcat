// ----- standard library imports
// ----- extra library imports
use axum::{extract::FromRef, routing::get, Router};
use utoipa::OpenApi;
// ----- local modules
mod admin;
mod error;
mod persistence;
mod service;
// ----- local imports
use error::Result;

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;

type ProdDB = persistence::surreal::DBBalance;
type ProdService = service::Service<ProdDB>;

#[derive(Debug, serde::Deserialize)]
pub struct AppConfig {
    pub treasury_url: bcr_wdc_treasury_client::Url,
    pub ebpp_url: bcr_wdc_ebpp_client::Url,
    pub eiou_url: bcr_wdc_eiou_client::Url,
    pub db_config: persistence::surreal::DBConfig,
}

#[derive(Debug, Clone, FromRef)]
pub struct AppController {
    pub service: ProdService,
}

impl AppController {
    pub async fn new(config: AppConfig) -> Result<Self> {
        let db = persistence::surreal::DBBalance::new(config.db_config).await?;
        let treasury = bcr_wdc_treasury_client::TreasuryClient::new(config.treasury_url);
        let ebpp = bcr_wdc_ebpp_client::EBPPClient::new(config.ebpp_url);
        let eiou = bcr_wdc_eiou_client::EIOUClient::new(config.eiou_url);

        let service = ProdService {
            treasury,
            ebpp,
            eiou,
            db,
        };

        Ok(Self { service })
    }

    pub async fn collect_balances(&self, tstamp: TStamp) -> Result<()> {
        self.service.collect_crsat_balance(tstamp).await?;
        self.service.collect_sat_balance(tstamp).await?;
        self.service.collect_onchain_balance(tstamp).await?;
        self.service.collect_eiou_balance(tstamp).await?;
        Ok(())
    }
}

pub fn routes(ctrl: AppController) -> Router {
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    let admin = Router::new()
        .route("/v1/admin/crsat/chart", get(admin::crsat_chart))
        .route("/v1/admin/sat/chart", get(admin::sat_chart))
        .route("/v1/admin/btc/chart", get(admin::btc_chart))
        .route("/v1/admin/eiou/chart", get(admin::eiou_chart));

    Router::new().merge(admin).with_state(ctrl).merge(swagger)
}

#[derive(utoipa::OpenApi)]
#[openapi(
    components(schemas(
        bcr_wdc_webapi::wallet::Balance,
        bcr_wdc_webapi::wallet::Candle,
        bcr_wdc_webapi::wallet::CandleChart,
        bcr_wdc_webapi::wallet::ECashBalance,
    ),),
    paths(
        admin::crsat_chart,
        admin::sat_chart,
        admin::btc_chart,
        admin::eiou_chart,
    )
)]
struct ApiDoc;
