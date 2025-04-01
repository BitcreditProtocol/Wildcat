// ----- standard library imports
// ----- extra library imports
use anyhow::anyhow;
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
    #[error("Mnemonic to xpriv conversion failed")]
    MnemonicToXpriv,
}

impl std::convert::From<Error> for cdk_common::payment::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::MnemonicToXpriv => Self::Anyhow(anyhow!(e)),
            Error::BDKEmptyOption(s) => Self::Anyhow(anyhow!(s)),
            Error::BDKCreateWithPersisted(e) => Self::Anyhow(anyhow!(e)),
            Error::BDKLoadWithPersisted(e) => Self::Anyhow(anyhow!(e)),
            Error::BDKSQLite(e) => Self::Anyhow(anyhow!(e)),
            Error::BDKKey(e) => Self::Anyhow(anyhow!(e)),
        }
    }
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        let resp = match self {
            Error::MnemonicToXpriv => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::BDKEmptyOption(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::BDKCreateWithPersisted(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::BDKLoadWithPersisted(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::BDKSQLite(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
            Error::BDKKey(_) => (StatusCode::INTERNAL_SERVER_ERROR, String::new()),
        };
        resp.into_response()
    }
}
