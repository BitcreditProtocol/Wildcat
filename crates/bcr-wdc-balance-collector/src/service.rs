// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bdk_wallet::bitcoin as btc;
// ----- local imports
use crate::{error::Result, TStamp};

// ----- end imports

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Candle<Amount> {
    pub tstamp: TStamp,
    pub open: Amount,
    pub high: Amount,
    pub low: Amount,
    pub close: Amount,
}
#[async_trait]
pub trait BalanceRepository {
    async fn store_crsat(&self, tstamp: TStamp, balance: cashu::Amount) -> Result<()>;
    async fn store_sat(&self, tstamp: TStamp, balance: cashu::Amount) -> Result<()>;
    async fn store_onchain(&self, tstamp: TStamp, balance: btc::Amount) -> Result<()>;
    async fn store_eiou(&self, tstamp: TStamp, balance: btc::Amount) -> Result<()>;

    async fn crsat_chart(&self, from: TStamp, to: TStamp) -> Result<Vec<Candle<cashu::Amount>>>;
    async fn sat_chart(&self, from: TStamp, to: TStamp) -> Result<Vec<Candle<cashu::Amount>>>;
    async fn onchain_chart(&self, from: TStamp, to: TStamp) -> Result<Vec<Candle<btc::Amount>>>;
    async fn eiou_chart(&self, from: TStamp, to: TStamp) -> Result<Vec<Candle<btc::Amount>>>;
}

#[derive(Debug, Clone)]
pub struct Service<DB> {
    pub treasury: bcr_wdc_treasury_client::TreasuryClient,
    pub ebpp: bcr_wdc_ebpp_client::EBPPClient,
    pub eiou: bcr_wdc_eiou_client::EIOUClient,
    pub db: DB,
}

impl<DB> Service<DB>
where
    DB: BalanceRepository + Send + Sync,
{
    pub async fn collect_crsat_balance(&self, tstamp: TStamp) -> Result<()> {
        let balance = self.treasury.crsat_balance().await?;
        self.db.store_crsat(tstamp, balance.amount).await
    }

    pub async fn collect_sat_balance(&self, tstamp: TStamp) -> Result<()> {
        let balance = self.treasury.sat_balance().await?;
        self.db.store_sat(tstamp, balance.amount).await
    }

    pub async fn collect_onchain_balance(&self, tstamp: TStamp) -> Result<()> {
        let balance = self.ebpp.balance().await?;
        self.db.store_onchain(tstamp, balance.confirmed).await
    }

    pub async fn collect_eiou_balance(&self, tstamp: TStamp) -> Result<()> {
        let balance = self.eiou.balance().await?;
        self.db.store_eiou(tstamp, balance.treasury).await
    }

    pub async fn query_crsat_chart(
        &self,
        from: TStamp,
        to: TStamp,
    ) -> Result<Vec<Candle<cashu::Amount>>> {
        let candles = self.db.crsat_chart(from, to).await?;
        Ok(candles)
    }

    pub async fn query_sat_chart(
        &self,
        from: TStamp,
        to: TStamp,
    ) -> Result<Vec<Candle<cashu::Amount>>> {
        let candles = self.db.sat_chart(from, to).await?;
        Ok(candles)
    }

    pub async fn query_onchain_chart(
        &self,
        from: TStamp,
        to: TStamp,
    ) -> Result<Vec<Candle<btc::Amount>>> {
        let candles = self.db.onchain_chart(from, to).await?;
        Ok(candles)
    }

    pub async fn query_eiou_chart(
        &self,
        from: TStamp,
        to: TStamp,
    ) -> Result<Vec<Candle<btc::Amount>>> {
        let candles = self.db.eiou_chart(from, to).await?;
        Ok(candles)
    }
}
