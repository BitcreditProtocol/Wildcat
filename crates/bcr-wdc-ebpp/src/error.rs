// ----- standard library imports
// ----- extra library imports
use anyhow::Error as AnyError;
use axum::http::StatusCode;
use cdk_common::{amount::Error as CDKAmountError, payment::Error as CDKPaymentError};
use thiserror::Error;
use uuid::Uuid;
// ----- local imports

// ----- end imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("bdk_wallet::keys {0}")]
    BDKKey(#[from] bdk_wallet::keys::KeyError),
    #[error("bdk_wallet::rusqlite {0}")]
    BDKSQLite(#[from] bdk_wallet::rusqlite::Error),
    #[error("bdk_wallet::LoadWithPersisted {0}")]
    BDKLoadWithPersisted(#[from] bdk_wallet::LoadWithPersistError<bdk_wallet::rusqlite::Error>),
    #[error("bdk_wallet::CreateWithPersisted {0}")]
    BDKCreateWithPersisted(#[from] bdk_wallet::CreateWithPersistError<bdk_wallet::rusqlite::Error>),
    #[error("bdk_wallet::chain: {0}")]
    BDKCannotConnect(#[from] bdk_wallet::chain::local_chain::CannotConnectError),
    #[error("bdk_wallet::create tx: {0}")]
    BDKCreateTx(#[from] bdk_wallet::error::CreateTxError),
    #[error("bdk_wallet::signer: {0}")]
    BDKSigner(#[from] bdk_wallet::signer::SignerError),
    #[error("bdk_wallet::signer not ok")]
    BDKSignOpNotOK,

    #[error("bitcoin::psbt extract: {0}")]
    BTCPsbtExtract(#[from] bdk_wallet::bitcoin::psbt::ExtractTxError),
    #[error("bitcoin::address parse: {0}")]
    BTCAddressParse(#[from] bdk_wallet::bitcoin::address::ParseError),
    #[error("bitcoin::amount parse: {0}")]
    BTCAmountParse(#[from] bdk_wallet::bitcoin::amount::ParseAmountError),
    #[error("bitcoin::psbt: {0}")]
    BTCPsbt(#[from] bdk_wallet::bitcoin::psbt::Error),

    #[error("miniscript: {0}")]
    Miniscript(#[from] bdk_wallet::miniscript::Error),
    #[error("DB errror: {0}")]
    DB(AnyError),
    #[error("Mnemonic to xpriv conversion failed")]
    MnemonicToXpriv,
    #[error("chrono conversion: {0}")]
    Chrono(#[from] chrono::OutOfRangeError),
    #[error("electrum_client: {0}")]
    Electrum(#[from] electrum_client::Error),
    #[error("tokio::spawn_blocking {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("bip21::from_str {0}")]
    Bip21Parse(AnyError),

    #[error("onchain wallet storage path error: {0}")]
    OnChainStore(std::path::PathBuf),

    #[error("payment request not found {0}")]
    PaymentRequestNotFound(Uuid),
    #[error("unknown address {0}")]
    UnknownAddress(bdk_wallet::bitcoin::Address),
    #[error("unknown amount")]
    UnknownAmount,
    #[error("tx_id not found {0}")]
    TxNotFound(bdk_wallet::bitcoin::Txid),
}

impl std::convert::From<Error> for cdk_common::payment::Error {
    fn from(e: Error) -> Self {
        tracing::error!("Error --> PaymentError: {:?}", e);
        match e {
            Error::TxNotFound(_) => CDKPaymentError::UnknownPaymentState,
            Error::UnknownAmount => CDKPaymentError::Amount(CDKAmountError::InvalidAmount(
                String::from("unknown amount"),
            )),
            Error::UnknownAddress(address) => {
                CDKPaymentError::Custom(format!("unknown bitcoin address {address}"))
            }
            Error::PaymentRequestNotFound(_) => CDKPaymentError::UnknownPaymentState,
            Error::Chrono(_) => CDKPaymentError::UnsupportedPaymentOption,
            Error::Bip21Parse(bip21_err) => CDKPaymentError::Anyhow(bip21_err),
            Error::DB(db_err) => CDKPaymentError::Anyhow(db_err),

            Error::Join(_) => CDKPaymentError::Custom(String::from("internal error")),
            Error::Electrum(_) => CDKPaymentError::Custom(String::from("internal error")),
            Error::BTCPsbt(_) => CDKPaymentError::Custom(String::from("internal error")),
            Error::BTCAmountParse(_) => CDKPaymentError::Custom(String::from("internal error")),
            Error::BTCAddressParse(_) => CDKPaymentError::Custom(String::from("internal error")),
            Error::BTCPsbtExtract(_) => CDKPaymentError::Custom(String::from("internal error")),
            Error::BDKSignOpNotOK => CDKPaymentError::Custom(String::from("internal error")),
            Error::BDKSQLite(_) => CDKPaymentError::Custom(String::from("internal error")),
            Error::BDKKey(_) => CDKPaymentError::Custom(String::from("internal error")),
            Error::BDKSigner(_) => CDKPaymentError::Custom(String::from("internal error")),
            Error::BDKCreateTx(_) => CDKPaymentError::Custom(String::from("internal error")),

            Error::Miniscript(_) => CDKPaymentError::Custom(String::from(
                "leaking internal error, this should never happen",
            )),
            Error::MnemonicToXpriv => CDKPaymentError::Custom(String::from(
                "leaking internal error, this should never happen",
            )),
            Error::OnChainStore(_) => CDKPaymentError::Custom(String::from(
                "leaking internal error, this should never happen",
            )),
            Error::BDKCannotConnect(_) => CDKPaymentError::Custom(String::from(
                "leaking internal error, this should never happen",
            )),
            Error::BDKCreateWithPersisted(_) => CDKPaymentError::Custom(String::from(
                "leaking internal error, this should never happen",
            )),
            Error::BDKLoadWithPersisted(_) => CDKPaymentError::Custom(String::from(
                "leaking internal error, this should never happen",
            )),
        }
    }
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("Error --> axum::Response: {:?}", self);
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
