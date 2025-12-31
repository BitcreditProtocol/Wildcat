// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use axum::{
    extract::{Json, Path, Query, State},
    http::header,
    response::{AppendHeaders, IntoResponse},
};
use bcr_common::{
    core::BillId,
    wire::{
        bill as wire_bill, clowder as wire_clowder, identity as wire_identity, keys as wire_keys,
        quotes as wire_quotes,
    },
};
// ----- local imports
use crate::{endpoints, error::Result, AppController};

// ----- end imports

#[utoipa::path(
    get,
    path = endpoints::HEALTH,
    params(
    ),
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_health() -> &'static str {
    "{ \"status\": \"OK\" }"
}

#[utoipa::path(
    get,
    path = endpoints::KEYSET_INFO,
    params(
        ("kid" = cashu::Id, Path, description = "the keyset id of the information")
    ),
    responses (
        (status = 200, description = "Successful response", body = cashu::KeySetInfo , content_type = "application/json"),
        (status = 404, description = "keyset id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_keyset_info(
    State(ctrl): State<AppController>,
    Path(kid): Path<cashu::Id>,
) -> Result<Json<cashu::KeySetInfo>> {
    tracing::debug!("Received keyset info request for {kid}");

    let info = ctrl.keys_cl.keyset_info(kid).await?;
    Ok(Json(info))
}

#[utoipa::path(
    get,
    path = endpoints::LIST_KEYSET_INFOS,
    params(
    ),
    responses (
        (status = 200, description = "Successful response", body = Vec<cashu::KeySetInfo> , content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_keyset_infos(
    State(ctrl): State<AppController>,
) -> Result<Json<Vec<cashu::KeySetInfo>>> {
    tracing::debug!("Received list keyset info request");

    let infos = ctrl.keys_cl.list_keyset_info().await?;
    Ok(Json(infos))
}

#[utoipa::path(
    get,
    path = endpoints::MINT_OP_STATUS,
    params(
        ("qid" = uuid::Uuid, Path, description = "the quote id this minting operation is associated with")
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_keys::MintOperationStatus , content_type = "application/json"),
        (status = 404, description = "resource id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_mintop_status(
    State(ctrl): State<AppController>,
    Path(qid): Path<uuid::Uuid>,
) -> Result<Json<wire_keys::MintOperationStatus>> {
    tracing::debug!("Received mint operation status request");

    let status = ctrl.keys_cl.mint_operation_status(qid).await?;
    Ok(Json(status))
}

#[utoipa::path(
    get,
    path = endpoints::LIST_MINT_OPS,
    params(
        ("kid" = cashu::Id, Path, description = "the keyset id the minting operations are associated with")
    ),
    responses (
        (status = 200, description = "Successful response", body = Vec<uuid::Uuid>, content_type = "application/json"),
        (status = 404, description = "keyset id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_mintops(
    State(ctrl): State<AppController>,
    Path(kid): Path<cashu::Id>,
) -> Result<Json<Vec<uuid::Uuid>>> {
    tracing::debug!("Received list mint operation request");

    let ids = ctrl.keys_cl.list_mint_operations(kid).await?;
    Ok(Json(ids))
}

#[utoipa::path(
    post,
    path = endpoints::ENABLE_REDEMPTION,
    request_body(content = wire_keys::DeactivateKeysetRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = wire_keys::DeactivateKeysetResponse , content_type = "application/json"),
        (status = 404, description = "keyset id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn post_enable_redemption(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_keys::DeactivateKeysetRequest>,
) -> Result<Json<wire_keys::DeactivateKeysetResponse>> {
    tracing::debug!("Received enable redemption request");

    let kid = ctrl.keys_cl.deactivate_keyset(request.kid).await?;
    Ok(Json(wire_keys::DeactivateKeysetResponse { kid }))
}

#[utoipa::path(
    get,
    path = endpoints::GET_CREDIT_QUOTE,
    params(
        ("qid" = uuid::Uuid, Path, description = "the quote id")
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_quotes::StatusReply , content_type = "application/json"),
        (status = 404, description = "quote id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_quote(
    State(ctrl): State<AppController>,
    Path(qid): Path<uuid::Uuid>,
) -> Result<Json<wire_quotes::StatusReply>> {
    tracing::debug!("Received credit quote request for {qid}");

    let status = ctrl.quotes_cl.lookup(qid).await?;
    Ok(Json(status))
}

#[utoipa::path(
    get,
    path = endpoints::LIST_CREDIT_QUOTES,
    params(wire_quotes::ListParam),
    responses (
        (status = 200, description = "Successful response", body = wire_quotes::ListReplyLight , content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_quotes(
    State(ctrl): State<AppController>,
    Query(list_params): Query<wire_quotes::ListParam>,
) -> Result<Json<wire_quotes::ListReplyLight>> {
    tracing::debug!("Received list quotes request");

    let statuss = ctrl.quotes_cl.list(list_params).await?;
    Ok(Json(statuss))
}

#[utoipa::path(
    put,
    path = endpoints::UPDATE_CREDIT_QUOTE,
    params(
        ("qid" = String, Path, description = "The quote id")
    ),
    request_body(content = wire_quotes::UpdateQuoteRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = wire_quotes::UpdateQuoteResponse, content_type = "application/json"),
        (status = 404, description = "quote id not found"),
    )
)]
pub async fn update_quote(
    State(ctrl): State<AppController>,
    Path(qid): Path<uuid::Uuid>,
    Json(req): Json<wire_quotes::UpdateQuoteRequest>,
) -> Result<Json<wire_quotes::UpdateQuoteResponse>> {
    tracing::debug!("Received mint quote update request");

    let response = match req {
        wire_quotes::UpdateQuoteRequest::Deny => ctrl.quotes_cl.deny(qid).await,
        wire_quotes::UpdateQuoteRequest::Offer { discounted, ttl } => {
            ctrl.quotes_cl.offer(qid, discounted, ttl).await
        }
    }?;
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = endpoints::ENABLE_QUOTE_MINTING,
    params(
        ("qid" = String, Path, description = "The quote id")
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_quotes::EnableMintingResponse , content_type = "application/json"),
        (status = 404, description = "quote id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn post_enable_quote_minting(
    State(ctrl): State<AppController>,
    Path(qid): Path<uuid::Uuid>,
) -> Result<Json<wire_quotes::EnableMintingResponse>> {
    tracing::debug!("Received enable quote minting request");

    let response = ctrl.quotes_cl.enable_minting(qid).await?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = endpoints::GET_IDENTITY,
    params(
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_identity::Identity , content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_identity(
    State(ctrl): State<AppController>,
) -> Result<Json<wire_identity::Identity>> {
    tracing::debug!("Received ebill identity request");

    let identity = ctrl.ebill_cl.get_identity().await?;
    Ok(Json(identity))
}

#[utoipa::path(
    get,
    path = endpoints::GET_EBILL,
    params(
        ("bid" = String, Path, description = "the ebill id")
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_bill::BitcreditBill , content_type = "application/json"),
        (status = 404, description = "bill id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_ebill(
    State(ctrl): State<AppController>,
    Path(bid): Path<BillId>,
) -> Result<Json<wire_bill::BitcreditBill>> {
    tracing::debug!("Received ebill info request for {bid}");

    let info = ctrl.ebill_cl.get_bill(&bid).await?;
    Ok(Json(info))
}

#[utoipa::path(
    get,
    path = endpoints::LIST_EBILLS,
    params(
    ),
    responses (
        (status = 200, description = "Successful response", body = Vec<wire_bill::BitcreditBill> , content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_ebills(
    State(ctrl): State<AppController>,
) -> Result<Json<Vec<wire_bill::BitcreditBill>>> {
    tracing::debug!("Received list ebill request");

    let infos = ctrl.ebill_cl.get_bills().await?;
    Ok(Json(infos))
}

#[utoipa::path(
    get,
    path = endpoints::GET_EBILL_ENDORSEMENTS,
    params(
        ("bid" = String, Path, description = "the ebill id")
    ),
    responses (
        (status = 200, description = "Successful response", body = Vec<wire_bill::Endorsement> , content_type = "application/json"),
        (status = 404, description = "bill id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_ebill_endorsements(
    State(ctrl): State<AppController>,
    Path(bid): Path<BillId>,
) -> Result<Json<Vec<wire_bill::Endorsement>>> {
    tracing::debug!("Received ebill endorsements request for {bid}");

    let endorsements = ctrl.ebill_cl.get_bill_endorsements(&bid).await?;
    Ok(Json(endorsements))
}

#[utoipa::path(
    get,
    path = endpoints::GET_EBILL_ATTACHMENT,
    params(
        ("bid" = String, Path, description = "the ebill id"),
        ("fname" = String, Path, description = "the file name")
    ),
    responses (
        (status = 200, description = "Successful response"),
        (status = 404, description = "bill-id/filename not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_ebill_attachment(
    State(ctrl): State<AppController>,
    Path((bid, fname_req)): Path<(BillId, String)>,
) -> impl IntoResponse {
    tracing::debug!("Received ebill info request for {bid}");

    let (fname_raw, raw) = ctrl.ebill_cl.get_bill_attachment(&bid, &fname_req).await?;
    let fname = std::path::PathBuf::from_str(&fname_raw).expect("PathBuf::from_str");
    let mime_type = match fname
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(|s| s.to_lowercase())
        .as_deref()
    {
        None => return Err(crate::error::Error::ResourceNotFound(fname_req)),
        Some("pdf") => mime::APPLICATION_PDF,
        Some("jpg") | Some("jpeg") => mime::IMAGE_JPEG,
        Some("png") => mime::IMAGE_PNG,
        Some(_) => mime::APPLICATION_OCTET_STREAM,
    };
    let headers = AppendHeaders([
        (header::CONTENT_TYPE, mime_type.to_string()),
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", fname_raw),
        ),
    ]);
    let stream = futures::stream::once(async move { Ok::<_, std::io::Error>(raw) });
    let body = axum::body::Body::from_stream(stream);
    Ok((headers, body))
}

#[utoipa::path(
    post,
    path = endpoints::POST_EBILL_REQTOPAY,
    request_body(content = wire_bill::RequestToPayBitcreditBillPayload, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response"),
        (status = 404, description = "bill id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn post_ebill_reqtopay(
    State(ctrl): State<AppController>,
    Json(req): Json<wire_bill::RequestToPayBitcreditBillPayload>,
) -> Result<()> {
    tracing::debug!("Received ebill request to pay for {}", req.bill_id);

    ctrl.ebill_cl.request_to_pay_bill(&req).await?;
    Ok(())
}

#[utoipa::path(
    get,
    path = endpoints::GET_CLOWDER_ALPHAS,
    params(
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_clowder::ConnectedMintsResponse , content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_clowder_alphas(
    State(ctrl): State<AppController>,
) -> Result<Json<wire_clowder::ConnectedMintsResponse>> {
    tracing::debug!("Received clowder alphas request");

    let response = ctrl.clwdr_cl.get_alphas().await?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = endpoints::GET_CLOWDER_BETAS,
    params(
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_clowder::ConnectedMintsResponse , content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_clowder_betas(
    State(ctrl): State<AppController>,
) -> Result<Json<wire_clowder::ConnectedMintsResponse>> {
    tracing::debug!("Received clowder betas request");

    let response = ctrl.clwdr_cl.get_betas().await?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = endpoints::GET_CLOWDER_MYSTATUS,
    params(
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_clowder::PerceivedState, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_clowder_mystatus(
    State(ctrl): State<AppController>,
) -> Result<Json<wire_clowder::PerceivedState>> {
    tracing::debug!("Received clowder mystatus request");

    let state = ctrl.clwdr_cl.get_mint_perceived_state().await?;
    Ok(Json(state))
}

#[utoipa::path(
    get,
    path = endpoints::GET_CLOWDER_STATUS,
    params(
        ("pk" = String, Path, description = "the public key of the mint to get the status for")
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_clowder::AlphaStateResponse, content_type = "application/json"),
        (status = 404, description = "public key not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_clowder_status(
    State(ctrl): State<AppController>,
    Path(pk): Path<secp256k1::PublicKey>,
) -> Result<Json<wire_clowder::AlphaStateResponse>> {
    tracing::debug!("Received clowder status request for {pk}");

    let state = ctrl.clwdr_cl.get_status(pk).await?;
    Ok(Json(state))
}
