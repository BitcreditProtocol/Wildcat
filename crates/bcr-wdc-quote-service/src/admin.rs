// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, Query, State};
use bcr_wdc_webapi::quotes as web_quotes;
// ----- local imports
use crate::error::Result;
use crate::quotes;
use crate::service::{calculate_default_expiration_date_for_quote, EBillNode};
use crate::service::{KeysHandler, ListFilters, Repository, Service, SortOrder, Wallet};

/// --------------------------- List quotes
#[utoipa::path(
    get,
    path = "/v1/admin/credit/quote/pending",
    params(
        ("since" = Option<chrono::NaiveDateTime>, Query, description = "only quote requests younger than `since`")
    ),
    responses (
        (status = 200, description = "Successful response", body = web_quotes::ListReply, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_pending_quotes<KeysHndlr, Wlt, QuotesRepo, EBillCl>(
    State(ctrl): State<Service<KeysHndlr, Wlt, QuotesRepo, EBillCl>>,
    Query(req): Query<web_quotes::ListPendingQueryRequest>,
) -> Result<Json<web_quotes::ListReply>>
where
    KeysHndlr: KeysHandler,
    Wlt: Wallet,
    QuotesRepo: Repository,
    EBillCl: EBillNode,
{
    tracing::debug!("Received request to list pending quotes");

    let quotes = ctrl.list_pendings(req.since).await?;
    Ok(Json(web_quotes::ListReply { quotes }))
}

fn convert_into_light_quote(quote: quotes::LightQuote) -> web_quotes::LightInfo {
    let status = match quote.status {
        quotes::StatusDiscriminants::Pending => web_quotes::StatusReplyDiscriminants::Pending,
        quotes::StatusDiscriminants::Canceled => web_quotes::StatusReplyDiscriminants::Canceled,
        quotes::StatusDiscriminants::Offered => web_quotes::StatusReplyDiscriminants::Offered,
        quotes::StatusDiscriminants::OfferExpired => {
            web_quotes::StatusReplyDiscriminants::OfferExpired
        }
        quotes::StatusDiscriminants::Denied => web_quotes::StatusReplyDiscriminants::Denied,
        quotes::StatusDiscriminants::Rejected => web_quotes::StatusReplyDiscriminants::Rejected,
        quotes::StatusDiscriminants::Accepted => web_quotes::StatusReplyDiscriminants::Accepted,
    };
    web_quotes::LightInfo {
        id: quote.id,
        status,
        sum: quote.sum,
    }
}

fn convert_into_list_params(params: web_quotes::ListParam) -> (ListFilters, Option<SortOrder>) {
    let web_quotes::ListParam {
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
        Some(web_quotes::StatusReplyDiscriminants::Pending) => {
            Some(quotes::StatusDiscriminants::Pending)
        }
        Some(web_quotes::StatusReplyDiscriminants::Canceled) => {
            Some(quotes::StatusDiscriminants::Canceled)
        }
        Some(web_quotes::StatusReplyDiscriminants::Offered) => {
            Some(quotes::StatusDiscriminants::Offered)
        }
        Some(web_quotes::StatusReplyDiscriminants::OfferExpired) => {
            Some(quotes::StatusDiscriminants::OfferExpired)
        }
        Some(web_quotes::StatusReplyDiscriminants::Denied) => {
            Some(quotes::StatusDiscriminants::Denied)
        }
        Some(web_quotes::StatusReplyDiscriminants::Rejected) => {
            Some(quotes::StatusDiscriminants::Rejected)
        }
        Some(web_quotes::StatusReplyDiscriminants::Accepted) => {
            Some(quotes::StatusDiscriminants::Accepted)
        }
    };
    let sort = match sort {
        None => None,
        Some(web_quotes::ListSort::BillMaturityDateDesc) => Some(SortOrder::BillMaturityDateDesc),
        Some(web_quotes::ListSort::BillMaturityDateAsc) => Some(SortOrder::BillMaturityDateAsc),
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

#[utoipa::path(
    get,
    path = "/v1/admin/credit/quote",
    params(web_quotes::ListParam),
    responses (
        (status = 200, description = "Successful response", body = web_quotes::ListReplyLight, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_quotes<KeysHndlr, Wlt, QuotesRepo, EBillCl>(
    State(ctrl): State<Service<KeysHndlr, Wlt, QuotesRepo, EBillCl>>,
    params: Query<web_quotes::ListParam>,
) -> Result<Json<web_quotes::ListReplyLight>>
where
    KeysHndlr: KeysHandler,
    Wlt: Wallet,
    QuotesRepo: Repository,
    EBillCl: EBillNode,
{
    tracing::debug!("Received request to list quotes");

    let now = chrono::Utc::now();
    let (filters, sort) = convert_into_list_params(params.0);
    let quotes = ctrl.list_light(filters, sort, now).await?;
    let response = web_quotes::ListReplyLight {
        quotes: quotes.into_iter().map(convert_into_light_quote).collect(),
    };
    Ok(Json(response))
}

/// --------------------------- Look up request
fn convert_to_info_reply(quote: quotes::Quote) -> web_quotes::InfoReply {
    match quote.status {
        quotes::Status::Pending { .. } => web_quotes::InfoReply::Pending {
            id: quote.id,
            bill: quote.bill.into(),
            submitted: quote.submitted,
            suggested_expiration: calculate_default_expiration_date_for_quote(chrono::Utc::now()),
        },
        quotes::Status::Canceled { tstamp } => web_quotes::InfoReply::Canceled {
            id: quote.id,
            bill: quote.bill.into(),
            tstamp,
        },
        quotes::Status::Offered {
            keyset_id,
            ttl,
            discounted,
        } => web_quotes::InfoReply::Offered {
            id: quote.id,
            bill: quote.bill.into(),
            discounted,
            ttl,
            keyset_id,
        },
        quotes::Status::OfferExpired { tstamp, discounted } => {
            web_quotes::InfoReply::OfferExpired {
                id: quote.id,
                bill: quote.bill.into(),
                discounted,
                tstamp,
            }
        }
        quotes::Status::Denied { tstamp } => web_quotes::InfoReply::Denied {
            id: quote.id,
            bill: quote.bill.into(),
            tstamp,
        },
        quotes::Status::Accepted {
            keyset_id,
            discounted,
        } => web_quotes::InfoReply::Accepted {
            id: quote.id,
            bill: quote.bill.into(),
            discounted,
            keyset_id,
        },
        quotes::Status::Rejected { tstamp, discounted } => web_quotes::InfoReply::Rejected {
            id: quote.id,
            bill: quote.bill.into(),
            discounted,
            tstamp,
        },
    }
}

#[utoipa::path(
    get,
    path = "/v1/admin/credit/quote/{id}",
    params(
        ("id" = String, Path, description = "The quote id")
    ),
    responses (
        (status = 200, description = "Successful response", body = web_quotes::InfoReply, content_type = "application/json"),
        (status = 404, description = "Quote id not  found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn admin_lookup_quote<KeysHndlr, Wlt, QuotesRepo, EBillCl>(
    State(ctrl): State<Service<KeysHndlr, Wlt, QuotesRepo, EBillCl>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<web_quotes::InfoReply>>
where
    KeysHndlr: KeysHandler,
    Wlt: Wallet,
    QuotesRepo: Repository,
    EBillCl: EBillNode,
{
    tracing::debug!("Received mint quote lookup request");

    let now = chrono::Utc::now();
    let quote = ctrl.lookup(id, now).await?;
    let response = convert_to_info_reply(quote);
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/v1/admin/credit/quote/{id}",
    params(
        ("id" = String, Path, description = "The quote id")
    ),
    request_body(content = web_quotes::UpdateQuoteRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = web_quotes::UpdateQuoteResponse, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn admin_update_quote<KeysHndlr, Wlt, QuotesRepo, EBillCl>(
    State(ctrl): State<Service<KeysHndlr, Wlt, QuotesRepo, EBillCl>>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<web_quotes::UpdateQuoteRequest>,
) -> Result<Json<web_quotes::UpdateQuoteResponse>>
where
    KeysHndlr: KeysHandler,
    Wlt: Wallet,
    QuotesRepo: Repository,
    EBillCl: EBillNode,
{
    tracing::debug!("Received mint quote update request");
    let now = chrono::Utc::now();
    let response = match req {
        web_quotes::UpdateQuoteRequest::Deny => {
            ctrl.deny(id, now).await?;
            web_quotes::UpdateQuoteResponse::Denied
        }
        web_quotes::UpdateQuoteRequest::Offer { discounted, ttl } => {
            let (discounted, ttl) = ctrl.offer(id, discounted, now, ttl).await?;
            web_quotes::UpdateQuoteResponse::Offered { discounted, ttl }
        }
    };
    Ok(Json(response))
}
