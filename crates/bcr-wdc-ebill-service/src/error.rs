// ----- standard library imports
// ----- extra library imports
use axum::http::StatusCode;
use bcr_ebill_api::service::{self, bill_service};
use thiserror::Error;
// ----- end imports

/// Generic result type
pub type Result<T> = std::result::Result<T, Error>;

/// Generic error type
#[derive(Debug, Error)]
pub enum Error {
    /// all errors originating from the bcr API service layer
    #[error("Service error: {0}")]
    Service(#[from] service::Error),

    /// all errors originating from the bcr API bill service layer
    #[error("Bill Service error: {0}")]
    BillService(#[from] service::bill_service::Error),

    /// all errors originating from the bcr API notification service layer
    #[error("Notification Service error: {0}")]
    NotificationService(#[from] bcr_ebill_transport::Error),

    /// all errors originating from validation
    #[error("Validation error: {0}")]
    Validation(#[from] bcr_ebill_api::util::ValidationError),
}

impl axum::response::IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        log::error!("Error: {self}");
        let response = match self {
            Error::Service(e) => ServiceError(e).into_response(),
            Error::BillService(e) => BillServiceError(e).into_response(),
            Error::NotificationService(e) => ServiceError(e.into()).into_response(),
            Error::Validation(e) => ValidationError(e).into_response(),
        };

        response.into_response()
    }
}

pub struct ServiceError(bcr_ebill_api::service::Error);

impl axum::response::IntoResponse for ServiceError {
    fn into_response(self) -> axum::response::Response {
        let response = match self.0 {
            bcr_ebill_api::service::Error::NoFileForFileUploadId => {
                (StatusCode::BAD_REQUEST, self.0.to_string()).into_response()
            }
            bcr_ebill_api::service::Error::NotFound => {
                (StatusCode::NOT_FOUND, "not found".to_string()).into_response()
            }
            bcr_ebill_api::service::Error::BillService(e) => BillServiceError(e).into_response(),
            bcr_ebill_api::service::Error::Validation(e) => ValidationError(e).into_response(),
            bcr_ebill_api::service::Error::ExternalApi(_)
            | bcr_ebill_api::service::Error::NotificationService(_)
            | bcr_ebill_api::service::Error::Io(_)
            | bcr_ebill_api::service::Error::CryptoUtil(_)
            | bcr_ebill_api::service::Error::Persistence(_)
            | bcr_ebill_api::service::Error::Blockchain(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, String::new()).into_response()
            }
        };
        response.into_response()
    }
}

pub struct BillServiceError(bill_service::Error);

impl axum::response::IntoResponse for BillServiceError {
    fn into_response(self) -> axum::response::Response {
        let response = match self.0 {
            bill_service::Error::NoFileForFileUploadId
            | bill_service::Error::DraweeNotInContacts
            | bill_service::Error::BuyerNotInContacts
            | bill_service::Error::EndorseeNotInContacts
            | bill_service::Error::MintNotInContacts
            | bill_service::Error::RecourseeNotInContacts
            | bill_service::Error::PayeeNotInContacts
            | bill_service::Error::InvalidOperation => {
                (StatusCode::BAD_REQUEST, self.0.to_string()).into_response()
            }
            bill_service::Error::Validation(validation_err) => {
                ValidationError(validation_err).into_response()
            }
            bill_service::Error::NotFound => {
                (StatusCode::NOT_FOUND, "not found".to_string()).into_response()
            }
            bill_service::Error::Io(_)
            | bill_service::Error::Persistence(_)
            | bill_service::Error::ExternalApi(_)
            | bill_service::Error::Blockchain(_)
            | bill_service::Error::Cryptography(_)
            | bill_service::Error::Notification(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, String::new()).into_response()
            }
        };
        response.into_response()
    }
}

pub struct ValidationError(bcr_ebill_api::util::ValidationError);

impl axum::response::IntoResponse for ValidationError {
    fn into_response(self) -> axum::response::Response {
        let response = match self.0 {
            bcr_ebill_api::util::ValidationError::RequestAlreadyExpired
                | bcr_ebill_api::util::ValidationError::FieldEmpty(_)
                | bcr_ebill_api::util::ValidationError::InvalidSum
                | bcr_ebill_api::util::ValidationError::InvalidCurrency
                | bcr_ebill_api::util::ValidationError::InvalidPaymentAddress
                | bcr_ebill_api::util::ValidationError::InvalidDate
                | bcr_ebill_api::util::ValidationError::IdentityCantBeAnon
                | bcr_ebill_api::util::ValidationError::IdentityIsNotBillIssuer
                | bcr_ebill_api::util::ValidationError::InvalidContact(_)
                | bcr_ebill_api::util::ValidationError::ContactIsAnonymous(_)
                | bcr_ebill_api::util::ValidationError::SignerCantBeAnon
                | bcr_ebill_api::util::ValidationError::IssueDateAfterMaturityDate
                | bcr_ebill_api::util::ValidationError::MaturityDateInThePast
                | bcr_ebill_api::util::ValidationError::InvalidFileUploadId
                | bcr_ebill_api::util::ValidationError::InvalidBillType
                | bcr_ebill_api::util::ValidationError::InvalidContentType
                | bcr_ebill_api::util::ValidationError::InvalidContactType
                | bcr_ebill_api::util::ValidationError::InvalidIdentityType
                | bcr_ebill_api::util::ValidationError::SelfDraftedBillCantBeBlank
                | bcr_ebill_api::util::ValidationError::DraweeCantBePayee
                | bcr_ebill_api::util::ValidationError::EndorserCantBeEndorsee
                | bcr_ebill_api::util::ValidationError::BuyerCantBeSeller
                | bcr_ebill_api::util::ValidationError::RecourserCantBeRecoursee
                | bcr_ebill_api::util::ValidationError::BillAlreadyAccepted
                | bcr_ebill_api::util::ValidationError::BillWasRejectedToAccept
                | bcr_ebill_api::util::ValidationError::BillAcceptanceExpired
                | bcr_ebill_api::util::ValidationError::BillWasRejectedToPay
                | bcr_ebill_api::util::ValidationError::BillPaymentExpired
                | bcr_ebill_api::util::ValidationError::BillWasRejectedToRecourse
                | bcr_ebill_api::util::ValidationError::BillRequestToRecourseExpired
                | bcr_ebill_api::util::ValidationError::BillWasRecoursedToTheEnd
                | bcr_ebill_api::util::ValidationError::BillWasNotOfferedToSell
                | bcr_ebill_api::util::ValidationError::BillWasNotRequestedToPay
                | bcr_ebill_api::util::ValidationError::BillWasNotRequestedToAccept
                | bcr_ebill_api::util::ValidationError::BillWasNotRequestedToRecourse
                | bcr_ebill_api::util::ValidationError::BillIsNotOfferToSellWaitingForPayment
                | bcr_ebill_api::util::ValidationError::BillIsOfferedToSellAndWaitingForPayment
                | bcr_ebill_api::util::ValidationError::BillWasRequestedToPay
                | bcr_ebill_api::util::ValidationError::BillIsInRecourseAndWaitingForPayment
                | bcr_ebill_api::util::ValidationError::BillRequestToAcceptDidNotExpireAndWasNotRejected
                | bcr_ebill_api::util::ValidationError::BillRequestToPayDidNotExpireAndWasNotRejected
                | bcr_ebill_api::util::ValidationError::BillIsNotRequestedToRecourseAndWaitingForPayment
                | bcr_ebill_api::util::ValidationError::BillSellDataInvalid
                | bcr_ebill_api::util::ValidationError::BillAlreadyPaid
                | bcr_ebill_api::util::ValidationError::BillNotAccepted
                | bcr_ebill_api::util::ValidationError::BillAlreadyRequestedToAccept
                | bcr_ebill_api::util::ValidationError::BillIsRequestedToPayAndWaitingForPayment
                | bcr_ebill_api::util::ValidationError::BillRecourseDataInvalid
                | bcr_ebill_api::util::ValidationError::RecourseeNotPastHolder
                | bcr_ebill_api::util::ValidationError::CallerIsNotDrawee
                | bcr_ebill_api::util::ValidationError::CallerIsNotBuyer
                | bcr_ebill_api::util::ValidationError::CallerIsNotRecoursee
                | bcr_ebill_api::util::ValidationError::RequestAlreadyRejected
                | bcr_ebill_api::util::ValidationError::BackupNotSupported
                | bcr_ebill_api::util::ValidationError::UnknownNodeId(_)
                | bcr_ebill_api::util::ValidationError::InvalidFileName(_)
                | bcr_ebill_api::util::ValidationError::FileIsTooBig(_)
                | bcr_ebill_api::util::ValidationError::InvalidSecp256k1Key(_)
                | bcr_ebill_api::util::ValidationError::NotASignatory(_)
                | bcr_ebill_api::util::ValidationError::SignatoryAlreadySignatory(_)
                | bcr_ebill_api::util::ValidationError::SignatoryNotInContacts(_)
                | bcr_ebill_api::util::ValidationError::CantRemoveLastSignatory
                | bcr_ebill_api::util::ValidationError::DrawerIsNotBillIssuer
                | bcr_ebill_api::util::ValidationError::CallerMustBeSignatory
                | bcr_ebill_api::util::ValidationError::CallerIsNotHolder

                => {
                    (StatusCode::BAD_REQUEST, self.0.to_string())
                },
            bcr_ebill_api::util::ValidationError::Blockchain(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, String::new())
            }
        };
        response.into_response()
    }
}
