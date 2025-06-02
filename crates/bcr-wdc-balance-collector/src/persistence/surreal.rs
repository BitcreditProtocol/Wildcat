// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bdk_wallet::bitcoin as btc;
use surrealdb::{engine::any::Any, Surreal};
// ----- local imports
use crate::{
    error::Result,
    service::{BalanceRepository, Candle},
    TStamp,
};

// ----- end imports

#[derive(Debug, serde::Deserialize)]
pub struct DBConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub crsat_table: String,
    pub sat_table: String,
    pub btc_table: String,
    pub eiou_table: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct AmountDBEntry<Amount> {
    amount: Amount,
}

#[derive(Debug, Clone)]
pub struct DBBalance {
    db: Surreal<Any>,
    crsat: String,
    sat: String,
    btc: String,
    eiou: String,
}
impl DBBalance {
    pub async fn new(config: DBConfig) -> surrealdb::Result<Self> {
        let db = Surreal::<Any>::init();
        db.connect(config.connection).await?;
        db.use_ns(config.namespace).await?;
        db.use_db(config.database).await?;

        Ok(Self {
            db,
            crsat: config.crsat_table,
            sat: config.sat_table,
            btc: config.btc_table,
            eiou: config.eiou_table,
        })
    }

    async fn store_amount<Amount>(
        &self,
        table: String,
        tstamp: TStamp,
        amount: Amount,
    ) -> Result<()>
    where
        Amount: serde::Serialize + serde::de::DeserializeOwned + 'static,
    {
        let statement = format!(
            "CREATE {table}:[d'{}']  CONTENT $entry",
            tstamp.to_rfc3339()
        );
        self.db
            .query(statement)
            .bind(("entry", AmountDBEntry { amount }))
            .await?;
        Ok(())
    }

    pub async fn chart<Amount>(
        &self,
        table: String,
        start: TStamp,
        end: TStamp,
    ) -> Result<Vec<Candle<Amount>>>
    where
        Amount: serde::de::DeserializeOwned,
    {
        let statement = format!(
            r#"SELECT time::group(id[0], "day") AS tstamp,
    array::first(amount) AS open, array::last(amount) AS close,
    math::max(amount) AS high, math::min(amount) AS low
    FROM {table}:[d'{}']..=[d'{}']
    GROUP BY tstamp"#,
            start.to_rfc3339(),
            end.to_rfc3339()
        );
        let candles: Vec<Candle<Amount>> = self
            .db
            .query(statement)
            .bind(("table", table))
            .bind(("start", start.timestamp()))
            .bind(("end", end.timestamp()))
            .await?
            .take(0)?;
        Ok(candles)
    }
}

#[async_trait]
impl BalanceRepository for DBBalance {
    async fn store_crsat(&self, tstamp: TStamp, amount: cashu::Amount) -> Result<()> {
        self.store_amount(self.crsat.clone(), tstamp, amount).await
    }

    async fn store_sat(&self, tstamp: TStamp, amount: cashu::Amount) -> Result<()> {
        self.store_amount(self.sat.clone(), tstamp, amount).await
    }

    async fn store_onchain(&self, tstamp: TStamp, amount: btc::Amount) -> Result<()> {
        self.store_amount(self.btc.clone(), tstamp, amount).await
    }

    async fn store_eiou(&self, tstamp: TStamp, amount: btc::Amount) -> Result<()> {
        self.store_amount(self.eiou.clone(), tstamp, amount).await
    }

    async fn crsat_chart(&self, from: TStamp, to: TStamp) -> Result<Vec<Candle<cashu::Amount>>> {
        self.chart(self.crsat.clone(), from, to).await
    }

    async fn sat_chart(&self, from: TStamp, to: TStamp) -> Result<Vec<Candle<cashu::Amount>>> {
        self.chart(self.sat.clone(), from, to).await
    }

    async fn onchain_chart(&self, from: TStamp, to: TStamp) -> Result<Vec<Candle<btc::Amount>>> {
        self.chart(self.btc.clone(), from, to).await
    }

    async fn eiou_chart(&self, from: TStamp, to: TStamp) -> Result<Vec<Candle<btc::Amount>>> {
        self.chart(self.eiou.clone(), from, to).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn init_mem_db() -> DBBalance {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBBalance {
            db: sdb,
            crsat: String::from("crsat"),
            sat: String::from("sat"),
            btc: String::from("btc"),
            eiou: String::from("eiou"),
        }
    }

    #[tokio::test]
    async fn store_and_chart_cashu_amount() {
        let db = init_mem_db().await;
        let tstamp = TStamp::from_timestamp(1735722000, 0).unwrap(); // 2025-01-01 10:00:00
        db.store_amount::<u64>(String::from("test"), tstamp, 1)
            .await
            .unwrap();
        let tstamp = TStamp::from_timestamp(1735808400, 0).unwrap(); // 2025-01-02 10:00:00
        db.store_amount::<u64>(String::from("test"), tstamp, 2)
            .await
            .unwrap();
        let tstamp = TStamp::from_timestamp(1735894800, 0).unwrap(); // 2025-01-03 10:00:00
        db.store_amount::<u64>(String::from("test"), tstamp, 3)
            .await
            .unwrap();

        let from = TStamp::from_timestamp(1735686000, 0).unwrap(); // 2025-01-01 00:00:00
        let to = TStamp::from_timestamp(1735858800, 0).unwrap(); // 2025-01-03 00:00:00
        let candles = db
            .chart::<u64>(String::from("test"), from, to)
            .await
            .unwrap();
        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].open, 1);
        assert_eq!(candles[0].close, 1);
        assert_eq!(candles[1].open, 2);
        assert_eq!(candles[1].close, 2);
    }
}
