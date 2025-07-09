// ----- standard library imports
// ----- extra library imports
use bdk_wallet::bitcoin as btc;
use cashu as cdk;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
// ----- local imports

// ----- end imports

///--------------------------- onchain wallet balance
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Balance {
    #[schema(value_type=u64)]
    pub immature: btc::Amount,
    #[schema(value_type=u64)]
    pub trusted_pending: btc::Amount,
    #[schema(value_type=u64)]
    pub untrusted_pending: btc::Amount,
    #[schema(value_type=u64)]
    pub confirmed: btc::Amount,
}

impl std::convert::From<bdk_wallet::Balance> for Balance {
    fn from(blnc: bdk_wallet::Balance) -> Self {
        Self {
            immature: blnc.immature,
            trusted_pending: blnc.trusted_pending,
            untrusted_pending: blnc.untrusted_pending,
            confirmed: blnc.confirmed,
        }
    }
}

impl std::convert::From<Balance> for bdk_wallet::Balance {
    fn from(blnc: Balance) -> Self {
        Self {
            immature: blnc.immature,
            trusted_pending: blnc.trusted_pending,
            untrusted_pending: blnc.untrusted_pending,
            confirmed: blnc.confirmed,
        }
    }
}

///--------------------------- eCash wallet balance
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ECashBalance {
    pub amount: cdk::Amount,
    pub unit: cdk::CurrencyUnit,
}

///--------------------------- eCash balance chart
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Candle {
    pub date: chrono::DateTime<chrono::Utc>,
    pub open: u64,
    pub high: u64,
    pub low: u64,
    pub close: u64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CandleChart {
    pub candles: Vec<Candle>,
}

///--------------------------- ebpp onchain network type
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Network {
    #[schema(value_type=String)]
    pub network: bitcoin::Network,
}
