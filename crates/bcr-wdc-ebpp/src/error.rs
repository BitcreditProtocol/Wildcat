// ----- standard library imports
// ----- extra library imports
use anyhow::Error as AnyError;
use axum::http::StatusCode;
use thiserror::Error;
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

    #[error("full_scan error: {0}")]
    EsploraFullScan(AnyError),
    #[error("sync error: {0}")]
    EsploraSync(AnyError),

    #[error("DB errror: {0}")]
    DB(AnyError),

    #[error("Mnemonic to xpriv conversion failed")]
    MnemonicToXpriv,

    #[error("onchain wallet storage path error: {0}")]
    OnChainStore(std::path::PathBuf),

    #[error("chrono conversion: {0}")]
    Chrono(chrono::OutOfRangeError),
}

impl std::convert::From<Error> for cdk_common::payment::Error {
    fn from(e: Error) -> Self {
        match e {
            _ => unreachable!("this should never be called"),
        }
    }
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        let resp = match self {
            _ => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };
        resp.into_response()
    }
}
