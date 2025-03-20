// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, Path, Query, State};
use bcr_wdc_webapi::quotes as web_quotes;
// ----- local imports
use crate::error::Result;
use crate::quotes;
use crate::service::{KeysHandler, ListFilters, Repository, Service, SortOrder, Wallet};
use crate::utils;

/// --------------------------- List quotes
#[utoipa::path(
    get,
    path = "/v1/admin/credit/quote/pending",
    params(
        ("since" = Option<chrono::NaiveDateTime>, Query, description = "only quote requests younger than `since`")
    ),
    responses (
        (status = 200, description = "Successful response", body = ListReply, content_type = "application/json"),
    )
)]
pub async fn list_pending_quotes<KeysHndlr, Wlt, QuotesRepo>(
    State(ctrl): State<Service<KeysHndlr, Wlt, QuotesRepo>>,
    since: Option<Query<chrono::DateTime<chrono::Utc>>>,
) -> Result<Json<web_quotes::ListReply>>
where
    KeysHndlr: KeysHandler,
    Wlt: Wallet,
    QuotesRepo: Repository,
{
    log::debug!("Received request to list pending quotes");

    let quotes = ctrl.list_pendings(since.map(|q| q.0)).await?;
    Ok(Json(web_quotes::ListReply { quotes }))
}

fn convert_into_light_quote(quote: quotes::LightQuote) -> web_quotes::LightInfo {
    let status = match quote.status {
        quotes::QuoteStatusDiscriminants::Pending => web_quotes::StatusReplyDiscriminants::Pending,
        quotes::QuoteStatusDiscriminants::Offered => web_quotes::StatusReplyDiscriminants::Offered,
        quotes::QuoteStatusDiscriminants::Denied => web_quotes::StatusReplyDiscriminants::Denied,
        quotes::QuoteStatusDiscriminants::Rejected => {
            web_quotes::StatusReplyDiscriminants::Rejected
        }
        quotes::QuoteStatusDiscriminants::Accepted => {
            web_quotes::StatusReplyDiscriminants::Accepted
        }
    };
    web_quotes::LightInfo {
        id: quote.id,
        status,
    }
}

fn convert_into_list_filters(filters: web_quotes::ListFilterParam) -> ListFilters {
    let web_quotes::ListFilterParam {
        bill_maturity_date_from,
        bill_maturity_date_to,
        status,
        bill_drawee_id,
        bill_drawer_id,
        bill_payer_id,
        bill_holder_id,
    } = filters;
    let status = match status {
        None => None,
        Some(web_quotes::StatusReplyDiscriminants::Pending) => {
            Some(quotes::QuoteStatusDiscriminants::Pending)
        }
        Some(web_quotes::StatusReplyDiscriminants::Offered) => {
            Some(quotes::QuoteStatusDiscriminants::Offered)
        }
        Some(web_quotes::StatusReplyDiscriminants::Denied) => {
            Some(quotes::QuoteStatusDiscriminants::Denied)
        }
        Some(web_quotes::StatusReplyDiscriminants::Rejected) => {
            Some(quotes::QuoteStatusDiscriminants::Rejected)
        }
        Some(web_quotes::StatusReplyDiscriminants::Accepted) => {
            Some(quotes::QuoteStatusDiscriminants::Accepted)
        }
    };
    ListFilters {
        bill_maturity_date_from,
        bill_maturity_date_to,
        status,
        bill_drawee_id,
        bill_drawer_id,
        bill_payer_id,
        bill_holder_id,
    }
}

fn convert_into_list_sort(sort: Option<web_quotes::ListSort>) -> Option<SortOrder> {
    match sort {
        None => None,
        Some(web_quotes::ListSort::BillMaturityDateDesc) => Some(SortOrder::BillMaturityDateDesc),
        Some(web_quotes::ListSort::BillMaturityDateAsc) => Some(SortOrder::BillMaturityDateAsc),
    }
}

#[utoipa::path(
    get,
    path = "/v1/admin/credit/quote",
    params(
        ("since" = Option<chrono::DateTime<chrono::Utc>>, Query, description = "quotes younger than `since`")
    ),
    responses (
        (status = 200, description = "Successful response", body = ListReplyLight, content_type = "application/json"),
    )
)]
pub async fn list_quotes<KeysHndlr, Wlt, QuotesRepo>(
    State(ctrl): State<Service<KeysHndlr, Wlt, QuotesRepo>>,
    Query(filters): Query<web_quotes::ListFilterParam>,
    sort: Option<Query<web_quotes::ListSort>>,
) -> Result<Json<web_quotes::ListReplyLight>>
where
    KeysHndlr: KeysHandler,
    Wlt: Wallet,
    QuotesRepo: Repository,
{
    log::debug!("Received request to list quotes");

    let filters = convert_into_list_filters(filters);
    let sort = convert_into_list_sort(sort.map(|q| q.0));
    let quotes = ctrl.list_light(filters, sort).await?;
    let response = web_quotes::ListReplyLight {
        quotes: quotes.into_iter().map(convert_into_light_quote).collect(),
    };
    Ok(Json(response))
}

/// --------------------------- Look up request
fn convert_to_info_reply(quote: quotes::Quote) -> web_quotes::InfoReply {
    match quote.status {
        quotes::QuoteStatus::Pending { .. } => web_quotes::InfoReply::Pending {
            id: quote.id,
            bill: quote.bill.into(),
            submitted: quote.submitted,
            suggested_expiration: utils::calculate_default_expiration_date_for_quote(
                chrono::Utc::now(),
            ),
        },
        quotes::QuoteStatus::Offered { signatures, ttl } => web_quotes::InfoReply::Offered {
            id: quote.id,
            bill: quote.bill.into(),
            ttl,
            signatures: signatures.clone(),
        },
        quotes::QuoteStatus::Denied => web_quotes::InfoReply::Denied {
            id: quote.id,
            bill: quote.bill.into(),
        },
        quotes::QuoteStatus::Accepted { signatures } => web_quotes::InfoReply::Accepted {
            id: quote.id,
            bill: quote.bill.into(),
            signatures,
        },
        quotes::QuoteStatus::Rejected { tstamp } => web_quotes::InfoReply::Rejected {
            id: quote.id,
            bill: quote.bill.into(),
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
        (status = 200, description = "Successful response", body = InfoReply, content_type = "application/json"),
        (status = 404, description = "Quote id not  found"),
    )
)]
pub async fn admin_lookup_quote<KeysHndlr, Wlt, QuotesRepo>(
    State(ctrl): State<Service<KeysHndlr, Wlt, QuotesRepo>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<web_quotes::InfoReply>>
where
    KeysHndlr: KeysHandler,
    Wlt: Wallet,
    QuotesRepo: Repository,
{
    log::debug!("Received mint quote lookup request for id: {}", id);

    let quote = ctrl.lookup(id).await?;
    let response = convert_to_info_reply(quote);
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/v1/admin/credit/quote/{id}",
    params(
        ("id" = String, Path, description = "The quote id")
    ),
    request_body(content = ResolveRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response"),
    )
)]
pub async fn admin_update_quote<KeysHndlr, Wlt, QuotesRepo>(
    State(ctrl): State<Service<KeysHndlr, Wlt, QuotesRepo>>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<web_quotes::UpdateQuoteRequest>,
) -> Result<Json<web_quotes::UpdateQuoteResponse>>
where
    KeysHndlr: KeysHandler,
    Wlt: Wallet,
    QuotesRepo: Repository,
{
    log::debug!("Received mint quote update request for id: {}", id);

    let response = match req {
        web_quotes::UpdateQuoteRequest::Deny => {
            ctrl.deny(id).await?;
            web_quotes::UpdateQuoteResponse::Denied
        }
        web_quotes::UpdateQuoteRequest::Offer { discount, ttl } => {
            let (discount, ttl) = ctrl.offer(id, discount, chrono::Utc::now(), ttl).await?;
            web_quotes::UpdateQuoteResponse::Offered { discount, ttl }
        }
    };
    Ok(Json(response))
}
