// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, Path, Query, State};
use bcr_common::wire::bill as wire_bill;
use bcr_common::wire::quotes as wire_quotes;
// ----- local imports
use crate::{
    error::{Error, Result},
    quotes,
    service::{calculate_default_expiration_date_for_quote, ListFilters, Service, SortOrder},
};

/// --------------------------- List quotes
fn convert_into_light_quote(quote: quotes::LightQuote) -> wire_quotes::LightInfo {
    let status = match quote.status {
        quotes::StatusDiscriminants::Pending => wire_quotes::InfoReplyDiscriminants::Pending,
        quotes::StatusDiscriminants::Canceled => wire_quotes::InfoReplyDiscriminants::Canceled,
        quotes::StatusDiscriminants::Offered => wire_quotes::InfoReplyDiscriminants::Offered,
        quotes::StatusDiscriminants::OfferExpired => {
            wire_quotes::InfoReplyDiscriminants::OfferExpired
        }
        quotes::StatusDiscriminants::Denied => wire_quotes::InfoReplyDiscriminants::Denied,
        quotes::StatusDiscriminants::Rejected => wire_quotes::InfoReplyDiscriminants::Rejected,
        quotes::StatusDiscriminants::Accepted => wire_quotes::InfoReplyDiscriminants::Accepted,
        quotes::StatusDiscriminants::MintingEnabled => {
            wire_quotes::InfoReplyDiscriminants::MintingEnabled
        }
        quotes::StatusDiscriminants::FailedEbillValidation => {
            wire_quotes::InfoReplyDiscriminants::FailedEbillValidation
        }
    };
    wire_quotes::LightInfo {
        id: quote.id,
        status,
        sum: quote.sum,
    }
}

fn convert_into_list_params(params: wire_quotes::ListParam) -> (ListFilters, Option<SortOrder>) {
    let wire_quotes::ListParam {
        bill_maturity_date_from,
        bill_maturity_date_to,
        status,
        bill_id,
        bill_drawee_id,
        bill_drawer_id,
        bill_payer_id,
        bill_holder_id,
        sort,
    } = params;
    let status = match status {
        None => None,
        Some(wire_quotes::InfoReplyDiscriminants::Pending) => {
            Some(quotes::StatusDiscriminants::Pending)
        }
        Some(wire_quotes::InfoReplyDiscriminants::Canceled) => {
            Some(quotes::StatusDiscriminants::Canceled)
        }
        Some(wire_quotes::InfoReplyDiscriminants::Offered) => {
            Some(quotes::StatusDiscriminants::Offered)
        }
        Some(wire_quotes::InfoReplyDiscriminants::OfferExpired) => {
            Some(quotes::StatusDiscriminants::OfferExpired)
        }
        Some(wire_quotes::InfoReplyDiscriminants::Denied) => {
            Some(quotes::StatusDiscriminants::Denied)
        }
        Some(wire_quotes::InfoReplyDiscriminants::Rejected) => {
            Some(quotes::StatusDiscriminants::Rejected)
        }
        Some(wire_quotes::InfoReplyDiscriminants::Accepted) => {
            Some(quotes::StatusDiscriminants::Accepted)
        }
        Some(wire_quotes::InfoReplyDiscriminants::MintingEnabled) => {
            Some(quotes::StatusDiscriminants::MintingEnabled)
        }
        Some(wire_quotes::InfoReplyDiscriminants::FailedEbillValidation) => {
            Some(quotes::StatusDiscriminants::FailedEbillValidation)
        }
    };
    let sort = match sort {
        None => None,
        Some(wire_quotes::ListSort::BillMaturityDateDesc) => Some(SortOrder::BillMaturityDateDesc),
        Some(wire_quotes::ListSort::BillMaturityDateAsc) => Some(SortOrder::BillMaturityDateAsc),
        Some(wire_quotes::ListSort::SubmittedDesc) => Some(SortOrder::SubmittedDesc),
        Some(wire_quotes::ListSort::SubmittedAsc) => Some(SortOrder::SubmittedAsc),
    };
    let filters = ListFilters {
        bill_maturity_date_from,
        bill_maturity_date_to,
        status,
        bill_id,
        bill_drawee_id,
        bill_drawer_id,
        bill_payer_id,
        bill_holder_id,
    };
    (filters, sort)
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_quotes(
    State(ctrl): State<Arc<Service>>,
    Query(params): Query<wire_quotes::ListParam>,
) -> Result<Json<wire_quotes::ListReplyLight>> {
    tracing::debug!("Received request to list quotes");

    let now = chrono::Utc::now();
    let (filters, sort) = convert_into_list_params(params);
    let quotes = ctrl.list_light(filters, sort, now).await?;
    let response = wire_quotes::ListReplyLight {
        quotes: quotes.into_iter().map(convert_into_light_quote).collect(),
    };
    Ok(Json(response))
}

/// --------------------------- Look up request
fn convert_to_info_reply(quote: quotes::Quote) -> wire_quotes::AdminInfoReply {
    let credit_program_version = quote
        .credit_program()
        .map(|binding| binding.version().to_owned());
    let credit_program_digest = quote
        .credit_program()
        .map(|binding| binding.digest().to_owned());
    let quote = match quote.status {
        quotes::Status::Pending { .. } => wire_quotes::InfoReply::Pending {
            id: quote.id,
            bill: wire_quotes::BillInfo::from(quote.bill),
            submitted: quote.submitted,
            suggested_expiration: calculate_default_expiration_date_for_quote(chrono::Utc::now()),
        },
        quotes::Status::Canceled { tstamp } => wire_quotes::InfoReply::Canceled {
            id: quote.id,
            bill: wire_quotes::BillInfo::from(quote.bill),
            tstamp,
        },
        quotes::Status::Offered {
            keyset_id,
            ttl,
            discounted,
            ..
        } => wire_quotes::InfoReply::Offered {
            id: quote.id,
            bill: wire_quotes::BillInfo::from(quote.bill),
            discounted,
            ttl,
            keyset_id,
        },
        quotes::Status::OfferExpired { tstamp, discounted } => {
            wire_quotes::InfoReply::OfferExpired {
                id: quote.id,
                bill: wire_quotes::BillInfo::from(quote.bill),
                discounted,
                tstamp,
            }
        }
        quotes::Status::Denied { tstamp } => wire_quotes::InfoReply::Denied {
            id: quote.id,
            bill: wire_quotes::BillInfo::from(quote.bill),
            tstamp,
        },
        quotes::Status::Accepted {
            keyset_id,
            discounted,
            ..
        } => wire_quotes::InfoReply::Accepted {
            id: quote.id,
            bill: wire_quotes::BillInfo::from(quote.bill),
            discounted,
            keyset_id,
        },
        quotes::Status::Rejected { tstamp, discounted } => wire_quotes::InfoReply::Rejected {
            id: quote.id,
            bill: wire_quotes::BillInfo::from(quote.bill),
            discounted,
            tstamp,
        },
        quotes::Status::MintingEnabled {
            keyset_id,
            fee,
            discounted,
            ..
        } => wire_quotes::InfoReply::MintingEnabled {
            id: quote.id,
            bill: wire_quotes::BillInfo::from(quote.bill),
            keyset_id,
            discounted,
            fee,
        },
        quotes::Status::FailedEbillValidation {
            keyset_id,
            discounted,
            ..
        } => wire_quotes::InfoReply::FailedEbillValidation {
            id: quote.id,
            bill: wire_quotes::BillInfo::from(quote.bill),
            discounted,
            keyset_id,
        },
    };
    wire_quotes::AdminInfoReply {
        quote,
        credit_program_version,
        credit_program_digest,
    }
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn lookup_quote(
    State(ctrl): State<Arc<Service>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<wire_quotes::AdminInfoReply>> {
    tracing::debug!("Received mint quote lookup request {id}");

    let now = chrono::Utc::now();
    let quote = ctrl.lookup(id, now).await?;
    let response = convert_to_info_reply(quote);
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn update_quote(
    State(ctrl): State<Arc<Service>>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<wire_quotes::UpdateQuoteRequest>,
) -> Result<Json<wire_quotes::UpdateQuoteResponse>> {
    tracing::debug!("Received mint quote update request");

    let now = chrono::Utc::now();
    let response = match req {
        wire_quotes::UpdateQuoteRequest::Deny => {
            ctrl.deny(id, now).await?;
            wire_quotes::UpdateQuoteResponse::Denied
        }
        wire_quotes::UpdateQuoteRequest::Offer { discounted, ttl } => {
            let _ = (discounted, ttl);
            return Err(Error::CreditAuthorizationRequired);
        }
    };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, request))]
pub async fn authorize_quote(
    State(ctrl): State<Arc<Service>>,
    Path(id): Path<uuid::Uuid>,
    Json(request): Json<wire_quotes::AuthorizedQuoteRequest>,
) -> Result<Json<wire_quotes::CreditAuthorizationReceipt>> {
    let receipt = ctrl
        .authorize_offer(id, request.signed_authorization, chrono::Utc::now())
        .await?;
    Ok(Json(receipt))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn enable_minting(
    State(ctrl): State<Arc<Service>>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<wire_quotes::EnableMintingRequest>,
) -> Result<Json<wire_quotes::EnableMintingResponse>> {
    tracing::debug!("Received enable mint for quote request");
    ctrl.enable_minting_manual_override(id).await?;
    let response = wire_quotes::EnableMintingResponse {};
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_shared_ebill_history(
    State(ctrl): State<Arc<Service>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<Vec<wire_bill::BillHistoryBlock>>> {
    tracing::debug!("Received get shared ebill history request");
    let bill_history_blocks = ctrl.get_shared_ebill_history(id).await?;
    Ok(Json(bill_history_blocks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_wdc_utils::keys::test_utils as keys_test;

    fn pending_quote() -> quotes::Quote {
        quotes::Quote::new(
            quotes::BillInfo::random(),
            keys_test::publics()[0],
            crate::TStamp::default(),
            quotes::test_credit_program_binding(),
        )
    }

    #[test]
    fn admin_quote_json_exposes_top_level_credit_program_binding() {
        let value = serde_json::to_value(convert_to_info_reply(pending_quote())).unwrap();

        assert_eq!(value["status"], "Pending");
        assert_eq!(value["credit_program_version"], "test-credit-program-v1");
        assert_eq!(
            value["credit_program_digest"],
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
        assert!(value.get("quote").is_none());
    }

    #[test]
    fn legacy_admin_quote_json_is_explicitly_unbound() {
        let mut quote = pending_quote();
        quote.credit_program = None;

        let value = serde_json::to_value(convert_to_info_reply(quote)).unwrap();

        assert!(value["credit_program_version"].is_null());
        assert!(value["credit_program_digest"].is_null());
    }
}
