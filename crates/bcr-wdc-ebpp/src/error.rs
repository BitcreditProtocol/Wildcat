// ----- standard library imports
// ----- extra library imports
use anyhow::Error as AnyError;
use axum::http::StatusCode;
use thiserror::Error;
use uuid::Uuid;
// ----- local imports

// ----- end imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("bdk_wallet::keys {0}")]
    BDKKey(bdk_wallet::keys::KeyError),
    #[error("bdk_wallet::rusqlite {0}")]
    BDKSQLite(bdk_wallet::rusqlite::Error),
    #[error("bdk_wallet::LoadWithPersisted {0}")]
    BDKLoadWithPersisted(bdk_wallet::LoadWithPersistError<bdk_wallet::rusqlite::Error>),
    #[error("bdk_wallet::CreateWithPersisted {0}")]
    BDKCreateWithPersisted(bdk_wallet::CreateWithPersistError<bdk_wallet::rusqlite::Error>),
    #[error("bdk_wallet:: empty Option on {0} call")]
    BDKEmptyOption(String),
    #[error("bdk_wallet::chain: {0}")]
    BDKCannotConnect(bdk_wallet::chain::local_chain::CannotConnectError),
    #[error("bitcoin::address parse: {0}")]
    BTCAddressParse(bdk_wallet::bitcoin::address::ParseError),
    #[error("miniscript: {0}")]
    Miniscript(bdk_wallet::miniscript::Error),
    #[error("full_scan error: {0}")]
    EsploraFullScan(AnyError),
    #[error("sync error: {0}")]
    EsploraSync(AnyError),
    #[error("DB errror: {0}")]
    DB(AnyError),
    #[error("Mnemonic to xpriv conversion failed")]
    MnemonicToXpriv,
    #[error("chrono conversion: {0}")]
    Chrono(chrono::OutOfRangeError),

    #[error("onchain wallet storage path error: {0}")]
    OnChainStore(std::path::PathBuf),

    #[error("payment request not found {0}")]
    PaymentRequestNotFound(Uuid),
    #[error("unknown address {0}")]
    UnknownAddress(bdk_wallet::bitcoin::Address),
}

impl std::convert::From<Error> for cdk_common::payment::Error {
    fn from(e: Error) -> Self {
        log::error!("Error --> PaymentError: {:?}", e);
        match e {
            Error::PaymentRequestNotFound(_) => cdk_common::payment::Error::UnknownPaymentState,
            _ => unreachable!("this should never be happening"),
        }
    }
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        log::error!("Error --> axum::Response: {:?}", self);
        let resp = match self {
            Error::PaymentRequestNotFound(reqid) => (
                StatusCode::NOT_FOUND,
                format!("Payment request not found {0}", reqid),
            ),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };
        resp.into_response()
    }
}
