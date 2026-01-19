// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, Query, State};
use bcr_common::wire::quotes as wire_quotes;
// ----- local imports
use crate::{
    error::Result,
    quotes,
    service::{self, calculate_default_expiration_date_for_quote, ListFilters, Service, SortOrder},
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
        quotes::StatusDiscriminants::MintingEnabled => wire_quotes::InfoReplyDiscriminants::Minting,
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
        Some(wire_quotes::InfoReplyDiscriminants::Minting) => {
            Some(quotes::StatusDiscriminants::MintingEnabled)
        }
    };
    let sort = match sort {
        None => None,
        Some(wire_quotes::ListSort::BillMaturityDateDesc) => Some(SortOrder::BillMaturityDateDesc),
        Some(wire_quotes::ListSort::BillMaturityDateAsc) => Some(SortOrder::BillMaturityDateAsc),
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
    State(ctrl): State<Service>,
    params: Query<wire_quotes::ListParam>,
) -> Result<Json<wire_quotes::ListReplyLight>> {
    tracing::debug!("Received request to list quotes");

    let now = chrono::Utc::now();
    let (filters, sort) = convert_into_list_params(params.0);
    let quotes = ctrl.list_light(filters, sort, now).await?;
    let response = wire_quotes::ListReplyLight {
        quotes: quotes.into_iter().map(convert_into_light_quote).collect(),
    };
    Ok(Json(response))
}

/// --------------------------- Look up request
fn convert_mint_status(status: service::MintingStatus) -> wire_quotes::MintingStatus {
    match status {
        service::MintingStatus::Disabled => wire_quotes::MintingStatus::Disabled,
        service::MintingStatus::Enabled(minted) => wire_quotes::MintingStatus::Enabled { minted },
    }
}
fn convert_to_info_reply(
    quote: quotes::Quote,
    minting_status: service::MintingStatus,
) -> wire_quotes::InfoReply {
    match quote.status {
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
        } => wire_quotes::InfoReply::Minting {
            id: quote.id,
            bill: wire_quotes::BillInfo::from(quote.bill),
            keyset_id,
            discounted,
            fee,
            minting_status: convert_mint_status(minting_status),
        },
    }
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn lookup_quote(
    State(ctrl): State<Service>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<wire_quotes::InfoReply>> {
    tracing::debug!("Received mint quote lookup request {id}");

    let now = chrono::Utc::now();
    let (quote, minting_status) = ctrl.lookup(id, now).await?;
    let response = convert_to_info_reply(quote, minting_status);
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn update_quote(
    State(ctrl): State<Service>,
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
            let (discounted, ttl) = ctrl.offer(id, discounted, now, ttl).await?;
            wire_quotes::UpdateQuoteResponse::Offered { discounted, ttl }
        }
    };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn enable_minting(
    State(ctrl): State<Service>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<wire_quotes::EnableMintingRequest>,
) -> Result<Json<wire_quotes::EnableMintingResponse>> {
    tracing::debug!("Received enable mint for quote request");

    ctrl.enable_minting(id).await?;
    let response = wire_quotes::EnableMintingResponse {};
    Ok(Json(response))
}
