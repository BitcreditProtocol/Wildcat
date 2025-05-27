// ----- standard library imports
use bcr_wdc_webapi::wallet as web_wallet;
use surrealdb::{engine::any::Any, Surreal};
// ----- local modules
mod error;
// ----- local imports
use error::Result;

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;

#[derive(Debug, serde::Deserialize)]
pub struct DBConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct AppConfig {
    pub treasury_url: bcr_wdc_treasury_client::Url,
    pub ebpp_url: bcr_wdc_ebpp_client::Url,
    pub eiou_url: bcr_wdc_eiou_client::Url,
    pub db_config: DBConfig,
}

pub struct AppController {
    pub treasury: bcr_wdc_treasury_client::TreasuryClient,
    pub ebpp: bcr_wdc_ebpp_client::EBPPClient,
    pub eiou: bcr_wdc_eiou_client::EIOUClient,
    pub db: surrealdb::Surreal<surrealdb::engine::any::Any>,
}

impl AppController {
    pub async fn new(config: AppConfig) -> Result<Self> {
        let AppConfig {
            treasury_url,
            ebpp_url,
            eiou_url,
            db_config,
        } = config;
        let db = Surreal::<Any>::init();
        db.connect(db_config.connection).await?;
        db.use_ns(db_config.namespace).await?;
        db.use_db(db_config.database).await?;

        let treasury = bcr_wdc_treasury_client::TreasuryClient::new(treasury_url);
        let ebpp = bcr_wdc_ebpp_client::EBPPClient::new(ebpp_url);
        let eiou = bcr_wdc_eiou_client::EIOUClient::new(eiou_url);
        Ok(Self {
            treasury,
            ebpp,
            eiou,
            db,
        })
    }

    async fn collect_crsat_balance(&self, tstamp: TStamp) -> Result<()> {
        let balance = self.treasury.crsat_balance().await?;
        let rid = surrealdb::RecordId::from_table_key("crsat", tstamp.timestamp());
        let _: Option<web_wallet::ECashBalance> = self.db.insert(rid).content(balance).await?;
        Ok(())
    }

    async fn collect_sat_balance(&self, tstamp: TStamp) -> Result<()> {
        let balance = self.treasury.sat_balance().await?;
        let rid = surrealdb::RecordId::from_table_key("sat", tstamp.timestamp());
        let _: Option<web_wallet::ECashBalance> = self.db.insert(rid).content(balance).await?;
        Ok(())
    }

    async fn collect_onchain_balance(&self, tstamp: TStamp) -> Result<()> {
        let balance = self.ebpp.balance().await?;
        let rid = surrealdb::RecordId::from_table_key("btc", tstamp.timestamp());
        let _: Option<bdk_wallet::Balance> = self.db.insert(rid).content(balance).await?;
        Ok(())
    }

    async fn collect_eiou_balance(&self, tstamp: TStamp) -> Result<()> {
        let balance = self.eiou.balance().await?;
        let rid = surrealdb::RecordId::from_table_key("eiou", tstamp.timestamp());
        let _: Option<bdk_wallet::Balance> = self.db.insert(rid).content(balance).await?;
        Ok(())
    }

    pub async fn collect_balances(&self, tstamp: TStamp) -> Result<()> {
        self.collect_crsat_balance(tstamp).await?;
        self.collect_sat_balance(tstamp).await?;
        self.collect_onchain_balance(tstamp).await?;
        self.collect_eiou_balance(tstamp).await?;
        Ok(())
    }
}
