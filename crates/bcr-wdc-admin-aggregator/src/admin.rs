// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use axum::{
    extract::{Json, Path, Query, State},
    http::header,
    response::{AppendHeaders, IntoResponse},
};
use bcr_common::{
    cashu::{self, ProofsMethods},
    client::ebill::Error as EbillClientError,
    core::BillId,
    wire::{
        bill as wire_bill, clowder as wire_clowder, common as wire_common,
        identity as wire_identity, info as wire_info, keys as wire_keys, quotes as wire_quotes,
        treasury as wire_treasury,
    },
};
use wire_clowder::ClowderNodeInfo;
// ----- local imports
use crate::{
    endpoints,
    error::{Error, Result},
    types, AppController,
};

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

    let info = ctrl.core_cl.keyset_info(kid).await?;
    Ok(Json(info))
}

#[utoipa::path(
    get,
    path = endpoints::LIST_KEYSET_INFOS,
    params(wire_common::Pagination),
    responses (
        (status = 200, description = "Successful response", body = wire_common::PaginatedResponse<cashu::KeySetInfo> , content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_keyset_infos(
    State(ctrl): State<AppController>,
    Query(pagination): Query<wire_common::Pagination>,
) -> Result<Json<wire_common::PaginatedResponse<cashu::KeySetInfo>>> {
    tracing::debug!("Received list keyset info request");

    let infos = ctrl.core_cl.list_keyset_info(Default::default()).await?;
    let total = infos.len() as u64;
    let data = if let Some(limit) = pagination.limit {
        let offset = pagination.offset.unwrap_or(0);
        let start = (offset as usize).min(infos.len());
        let end = (start + limit as usize).min(infos.len());
        infos[start..end].to_vec()
    } else {
        infos
    };
    Ok(Json(wire_common::PaginatedResponse { data, total }))
}

#[utoipa::path(
    get,
    path = endpoints::MINT_OP_STATUS,
    params(
        ("qid" = uuid::Uuid, Path, description = "the quote id this minting operation is associated with")
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_treasury::MintOperationStatus , content_type = "application/json"),
        (status = 404, description = "resource id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_mintop_status(
    State(ctrl): State<AppController>,
    Path(qid): Path<uuid::Uuid>,
) -> Result<Json<wire_treasury::MintOperationStatus>> {
    tracing::debug!("Received mint operation status request");

    let status = ctrl.treasury_cl.ebill_mint_operation_status(qid).await?;
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

    let ids = ctrl.treasury_cl.list_ebill_mint_operations(kid).await?;
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

    let kid = ctrl.core_cl.deactivate_keyset(request.kid).await?;
    Ok(Json(wire_keys::DeactivateKeysetResponse { kid }))
}

#[utoipa::path(
    get,
    path = endpoints::GET_CREDIT_QUOTE,
    params(
        ("qid" = uuid::Uuid, Path, description = "the quote id")
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_quotes::InfoReply , content_type = "application/json"),
        (status = 404, description = "quote id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_quote(
    State(ctrl): State<AppController>,
    Path(qid): Path<uuid::Uuid>,
) -> Result<Json<wire_quotes::InfoReply>> {
    tracing::debug!("Received credit quote request for {qid}");

    let status = ctrl.quotes_cl.lookup(qid).await?;
    Ok(Json(status))
}

#[utoipa::path(
    get,
    path = endpoints::LIST_CREDIT_QUOTES,
    params(wire_quotes::ListParam, wire_common::Pagination),
    responses (
        (status = 200, description = "Successful response", body = wire_common::PaginatedResponse<wire_quotes::LightInfo> , content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn list_quotes(
    State(ctrl): State<AppController>,
    Query(list_params): Query<wire_quotes::ListParam>,
    Query(pagination): Query<wire_common::Pagination>,
) -> Result<Json<wire_common::PaginatedResponse<wire_quotes::LightInfo>>> {
    tracing::debug!("Received list quotes request");

    let result = ctrl.quotes_cl.list(list_params).await?;
    let total = result.quotes.len() as u64;
    let data = if let Some(limit) = pagination.limit {
        let offset = pagination.offset.unwrap_or(0);
        let start = (offset as usize).min(result.quotes.len());
        let end = (start + limit as usize).min(result.quotes.len());
        result.quotes[start..end].to_vec()
    } else {
        result.quotes
    };
    Ok(Json(wire_common::PaginatedResponse { data, total }))
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
) -> Result<impl IntoResponse> {
    tracing::debug!("Received ebill attachment request for {bid}");

    let (content_type, raw) = ctrl.ebill_cl.get_bill_attachment(&bid, &fname_req).await?;
    let headers = AppendHeaders([
        (header::CONTENT_TYPE, content_type),
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", fname_req),
        ),
    ]);
    let stream = futures::stream::once(async move { Ok::<_, std::io::Error>(raw) });
    let body = axum::body::Body::from_stream(stream);
    Ok((headers, body))
}

#[utoipa::path(
    get,
    path = endpoints::GET_EBILL_FILE_FROM_REQUEST_TO_MINT,
    params(wire_quotes::RequestEncryptedFileUrlPayload),
    responses (
        (status = 200, description = "Successful response"),
        (status = 404, description = "file url not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_ebill_file_from_request_to_mint(
    State(ctrl): State<AppController>,
    Query(file_url_payload): Query<wire_quotes::RequestEncryptedFileUrlPayload>,
) -> Result<impl IntoResponse> {
    tracing::debug!(
        "Received ebill file from request to mint request for {}",
        file_url_payload.file_url
    );

    let (extension, content_type, raw) = ctrl
        .ebill_cl
        .get_file_from_request_to_mint(&file_url_payload.file_url)
        .await?;
    let headers = AppendHeaders([
        (header::CONTENT_TYPE, content_type),
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", format!("file.{}", extension)),
        ),
    ]);
    let stream = futures::stream::once(async move { Ok::<_, std::io::Error>(raw) });
    let body = axum::body::Body::from_stream(stream);
    Ok((headers, body))
}

#[utoipa::path(
    get,
    path = endpoints::GET_EBILL_PAYMENTSTATUS,
    params(
        ("bid" = String, Path, description = "the ebill id"),
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_bill::SimplifiedBillPaymentStatus, content_type = "application/json"),
        (status = 404, description = "bill-id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_ebill_paymentstatus(
    State(ctrl): State<AppController>,
    Path(bid): Path<BillId>,
) -> Result<Json<wire_bill::SimplifiedBillPaymentStatus>> {
    tracing::debug!("Received ebill payment status request for {bid}");

    let response = ctrl.ebill_cl.get_payment_status(bid).await;
    match response {
        Ok(status) => Ok(Json(status)),
        Err(EbillClientError::ResourceNotFound(resource)) => Err(Error::ResourceNotFound(resource)),
        Err(e) => Err(Error::EBillClient(e)),
    }
}

#[utoipa::path(
    get,
    path = endpoints::GET_CLOWDER_INFO,
    responses (
        (status = 200, description = "Successful response", body = wire_clowder::ClowderNodeInfo, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_clowder_info(
    State(ctrl): State<AppController>,
) -> Result<Json<wire_clowder::ClowderNodeInfo>> {
    tracing::debug!("Received clowder info request");

    let info = ctrl.clwdr_cl.get_info().await?;
    Ok(Json(info))
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

/// Returns coverage information about the local mint
#[utoipa::path(
    get,
    path = endpoints::GET_CLOWDER_LOCAL_COVERAGE,
    params(
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_clowder::Coverage , content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_clowder_local_coverage(
    State(ctrl): State<AppController>,
) -> Result<Json<wire_clowder::Coverage>> {
    tracing::debug!("Received clowder coverage request");

    let supply = ctrl.clwdr_cl.get_mint_circulating_supply().await?;
    let collateral = ctrl.clwdr_cl.get_mint_collateral().await?;

    Ok(Json(wire_clowder::Coverage {
        debit_circulating_supply: supply.debit,
        credit_circulating_supply: supply.credit,
        onchain_collateral: collateral.onchain,
        ebill_collateral: collateral.ebill,
        eiou_collateral: collateral.eiou,
    }))
}

/// Returns coverage information about an alpha mint this local mint is verifying in beta capacity
#[utoipa::path(
    get,
    path = endpoints::GET_CLOWDER_FOREIGN_COVERAGE,
    params(
        ("pk" = String, Path, description = "the public key of the mint to get the status for")
    ),
    responses (
        (status = 200, description = "Successful response", body = wire_clowder::Coverage , content_type = "application/json"),
        (status = 404, description = "public key not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_clowder_foreign_coverage(
    State(ctrl): State<AppController>,
    Path(pk): Path<secp256k1::PublicKey>,
) -> Result<Json<wire_clowder::Coverage>> {
    tracing::debug!("Received clowder coverage request");

    let supply = ctrl.clwdr_cl.get_circulating_supply(&pk).await?;
    let btc_amt = ctrl.clwdr_cl.get_collateral_onchain(&pk).await?.amount;
    let ebill_amt = ctrl.clwdr_cl.get_collateral_ebill(pk).await?.amount;
    let eiou_amt = ctrl.clwdr_cl.get_collateral_eiou(&pk).await?.amount;

    Ok(Json(wire_clowder::Coverage {
        debit_circulating_supply: supply.debit,
        credit_circulating_supply: supply.credit,
        onchain_collateral: btc_amt,
        ebill_collateral: ebill_amt,
        eiou_collateral: eiou_amt,
    }))
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

    let state = ctrl.clwdr_cl.get_status(&pk).await?;
    Ok(Json(state))
}

#[utoipa::path(
    post,
    path = endpoints::POST_EBILL_REQTOPAY,
    request_body(content = wire_treasury::RequestToPayFromEBillRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = wire_treasury::RequestToPayFromEBillResponse, content_type = "application/json"),
        (status = 404, description = "bill id not found"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn post_ebill_reqtopay(
    State(ctrl): State<AppController>,
    Json(req): Json<wire_treasury::RequestToPayFromEBillRequest>,
) -> Result<Json<wire_treasury::RequestToPayFromEBillResponse>> {
    tracing::debug!("Received ebill request to pay for {}", req.ebill_id);

    let response = ctrl
        .treasury_cl
        .request_to_pay_ebill(req.ebill_id, req.amount, req.deadline)
        .await?;
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = endpoints::MINT_INFO,
    responses (
        (status = 200, description = "Successful response", body = wire_info::WildcatInfo, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_mint_info(
    State(ctrl): State<AppController>,
) -> Result<Json<wire_info::WildcatInfo>> {
    tracing::debug!("Received request for wildcat info");

    let clwd_info = ctrl.clwdr_cl.get_info().await?;
    let ClowderNodeInfo {
        node_id,
        network,
        uptime_timestamp,
        change_address: clowder_change_address,
        version,
        multisig_agg_xonly: _,
    } = clwd_info;
    let build_time = bcr_wdc_utils::info::get_build_time();
    let uptime_timestamp = chrono::DateTime::from_timestamp(uptime_timestamp as i64, 0)
        .ok_or(Error::Internal(String::from("uptime_timestamp error")))?;
    let versions = wire_info::VersionInfo {
        bcr_ebill_core: bcr_wdc_utils::info::get_ebill_version()
            .map(|v| v.to_string())
            .unwrap_or(String::from("?")),
        clowder: version,
        wildcat: bcr_wdc_utils::info::get_version().to_string(),
        cdk_mintd: bcr_wdc_utils::info::get_cashu_version()
            .map(|v| v.to_string())
            .unwrap_or(String::from("?")),
    };
    let response = wire_info::WildcatInfo {
        build_time,
        clowder_change_address,
        network,
        clowder_node_id: *node_id,
        uptime_timestamp,
        versions,
    };
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = endpoints::POST_TOKEN_STATUS,
    request_body(content = types::TokenStateRequest, content_type = "application/json"),
    responses (
        (status = 200, description = "Successful response", body = types::TokenStateResponse, content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn post_token_status(
    State(ctrl): State<AppController>,
    Json(req): Json<types::TokenStateRequest>,
) -> Result<Json<types::TokenStateResponse>> {
    let head = req.token.chars().take(16).collect::<String>();
    tracing::debug!("Received token state request {}", head);

    let token = bcr_common::wallet::Token::from_str(&req.token)?;

    let kinfo_filters = wire_keys::KeysetInfoFilters {
        unit: token.unit(),
        ..Default::default()
    };
    let kinfos = ctrl.core_cl.list_keyset_info(kinfo_filters).await?;
    let ys = token.proofs(&kinfos)?.ys()?;
    let states = ctrl.core_cl.check_state(ys).await?;
    let is_any_spent = states
        .into_iter()
        .map(|s| s.state)
        .any(|s| matches!(s, cashu::State::Spent));
    if is_any_spent {
        Ok(Json(types::TokenStateResponse {
            state: types::TokenState::Spent,
        }))
    } else {
        Ok(Json(types::TokenStateResponse {
            state: types::TokenState::Unspent,
        }))
    }
}
