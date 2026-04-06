// ----- standard library imports
use std::{collections::HashSet, sync::Arc};
// ----- extra library imports
use async_trait::async_trait;
use axum::extract::{Json, Path, State};
use bcr_common::{
    cashu::{self, MintVersion},
    client::{
        clowder::Client as ClowderClient, core::Client as CoreClient,
        treasury::Client as TreasuryClient,
    },
    wire::{
        clowder::{self as wire_clowder, messages},
        exchange as wire_exchange,
        info::{VersionInfo, WildcatInfo},
        swap as wire_swap,
    },
};
use bitcoin::base64::{engine::general_purpose::STANDARD, Engine};
use futures::future::JoinAll;
// ----- local imports
use crate::{
    error::{Error, Result},
    AppController,
};

// ----- end imports

#[utoipa::path(
    get,
    path = "/health",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn health() -> Result<&'static str> {
    Ok("{ \"status\": \"OK\" }")
}

#[utoipa::path(
    get,
    path = "/v1/info",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_mint_info(State(ctrl): State<AppController>) -> Result<Json<cashu::MintInfo>> {
    tracing::debug!("Requested /v1/info");
    let network = ctrl.clwdr_rest_client.get_info().await?.network;
    let mut long_description = format!(
        r#"[clowder]
network = {network}
"#
    );
    let build_time = bcr_wdc_utils::info::get_build_time();
    let dep_versions = bcr_wdc_utils::info::get_deps_versions()
        .into_iter()
        .map(|(k, v)| {
            if v.is_some() {
                format!("{k} = {}", v.unwrap())
            } else {
                format!("{k} = ?")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    long_description += &format!(
        r#"
build-time = {build_time}
[versions]
{dep_versions}
"#,
    );
    let version = MintVersion {
        name: String::from("wildcat"),
        version: bcr_wdc_utils::info::get_version().to_string(),
    };
    let info = cashu::MintInfo {
        name: Some(String::from("bcr-wdc")),
        version: Some(version),
        description: Some(String::from("Wildcat One")),
        description_long: Some(long_description),
        ..Default::default()
    };
    Ok(Json(info))
}

#[utoipa::path(
    get,
    path = "/v1/wildcat",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_wildcat_info(State(ctrl): State<AppController>) -> Result<Json<WildcatInfo>> {
    tracing::debug!("Requested /v1/wildcat");

    let clowder_info = ctrl.clwdr_rest_client.get_info().await?;
    let network = clowder_info.network;
    let info = cashu::MintInfo::default();
    let build_time = bcr_wdc_utils::info::get_build_time();
    let ebill_core = bcr_wdc_utils::info::get_ebill_version()
        .map(|v| v.to_string())
        .unwrap_or(String::from("?"));
    let version = bcr_wdc_utils::info::get_version().to_string();
    let cdk_mintd = info
        .version
        .as_ref()
        .map(|v| v.version.clone())
        .unwrap_or(String::from("0.0.0"));

    let versions = VersionInfo {
        wildcat: version,
        bcr_ebill_core: ebill_core,
        cdk_mintd,
        clowder: clowder_info.version,
    };

    // Convert cashu::PublicKey to bitcoin::secp256k1::PublicKey (different secp256k1 versions)
    let node_id_bytes = clowder_info.node_id.to_bytes();
    let clowder_node_id = bitcoin::secp256k1::PublicKey::from_slice(&node_id_bytes)
        .map_err(|e| Error::Invalid(format!("Invalid node_id public key: {e}")))?;

    let wildcat_info = WildcatInfo {
        build_time,
        uptime_timestamp: ctrl.time_started,
        versions,
        network,
        clowder_node_id,
        clowder_change_address: clowder_info.change_address,
    };

    Ok(Json(wildcat_info))
}

#[utoipa::path(
    get,
    path = "/v1/keys",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_mint_keys(State(ctrl): State<AppController>) -> Result<Json<cashu::KeysResponse>> {
    tracing::debug!("Requested /v1/keys");

    let bcr_keys = ctrl.core_client.list_keys().await.unwrap_or_default();
    let response = cashu::KeysResponse { keysets: bcr_keys };
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/v1/keysets",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_mint_keysets(
    State(ctrl): State<AppController>,
) -> Result<Json<cashu::KeysetResponse>> {
    tracing::debug!("Requested /v1/keysets");

    let bcr_infos = ctrl
        .core_client
        .list_keyset_info(Default::default())
        .await
        .unwrap_or_default();
    let response = cashu::KeysetResponse { keysets: bcr_infos };
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/v1/keys/{kid}",
    params(
        ("kid" = cashu::Id, Path, description = "The keyset id")
    ),
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
        (status = 404, description = "Keyset not found"),
    )
)]
pub async fn get_mint_keyset(
    State(ctrl): State<AppController>,
    Path(kid): Path<cashu::Id>,
) -> Result<Json<cashu::KeysResponse>> {
    tracing::debug!("Requested /v1/keys/{}", kid);

    let keys = ctrl.core_client.keys(kid).await?;
    let response = cashu::KeysResponse {
        keysets: vec![keys],
    };
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/v1/keysets/{kid}",
    params(
        ("kid" = cashu::Id, Path, description = "The keyset id")
    ),
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
        (status = 404, description = "Keyset not found"),
    )
)]
pub async fn get_keyset_info(
    State(ctrl): State<AppController>,
    Path(kid): Path<cashu::Id>,
) -> Result<Json<cashu::KeySetInfo>> {
    tracing::debug!("Requested /v1/keysets/{}", kid);

    let info = ctrl.core_client.keyset_info(kid).await?;
    Ok(Json(info))
}

#[utoipa::path(
    post,
    path = "/v1/swap",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn post_swap(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_swap::SwapRequest>,
) -> Result<Json<wire_swap::SwapResponse>> {
    tracing::debug!("Requested /v1/swap");

    let now = chrono::Utc::now();
    ctrl.commit_srv
        .check_swap(now, &request.inputs, &request.outputs, &request.commitment)
        .await?;

    let wire_swap::SwapRequest {
        inputs,
        outputs,
        commitment,
    } = request;

    let input_type =
        determine_input_type(&ctrl.core_client, inputs.iter().map(|p| p.keyset_id)).await?;
    let htlc_unlocked = test_for_htlc(&inputs, input_type, &ctrl.treasury_client).await?;
    tracing::info!("HTLC unlocked in intermint exchange: {}", htlc_unlocked);

    let signatures = ctrl
        .core_client
        .swap(inputs.clone(), outputs.clone(), commitment)
        .await?;

    let req = messages::SwapRequest {
        proofs: inputs,
        blinds: outputs,
        commitment,
    };
    let resp = messages::SwapResponse {
        signatures: signatures.clone(),
    };
    ctrl.clwdr_stream_client.mint_swap(req, resp).await?;

    let response = wire_swap::SwapResponse { signatures };
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/v1/checkstate",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn post_check_state(
    State(ctrl): State<AppController>,
    Json(request): Json<cashu::CheckStateRequest>,
) -> Result<Json<cashu::CheckStateResponse>> {
    tracing::debug!("Requested /v1/checkstate");

    let credit_states = ctrl.core_client.check_state(request.ys.clone()).await?;
    Ok(Json(cashu::CheckStateResponse {
        states: credit_states,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/restore",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn post_restore(
    State(ctrl): State<AppController>,
    Json(request): Json<cashu::RestoreRequest>,
) -> Result<Json<cashu::RestoreResponse>> {
    tracing::debug!("Requested /v1/restore");

    let cashu::RestoreRequest { outputs } = request;
    let restore_pair = ctrl.core_client.restore(outputs).await?;
    let (outputs, signatures) = restore_pair.into_iter().unzip();
    let response = cashu::RestoreResponse {
        outputs,
        signatures,
        promises: Default::default(),
    };
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = ClowderClient::LOCAL_INFO_EP_V1,
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_clowder_info(
    State(ctrl): State<AppController>,
) -> Result<Json<wire_clowder::ClowderNodeInfo>> {
    Ok(Json(ctrl.clwdr_rest_client.get_info().await?))
}

#[utoipa::path(
    post,
    path = ClowderClient::LOCAL_PATH_EP_V1,
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn post_clowder_path(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_clowder::PathRequest>,
) -> Result<Json<wire_clowder::ConnectedMintsResponse>> {
    Ok(Json(
        ctrl.clwdr_rest_client
            .post_path(request.origin_mint_url)
            .await?,
    ))
}

#[utoipa::path(
    get,
    path = ClowderClient::LOCAL_BETAS_EP_V1,
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_clowder_betas(
    State(ctrl): State<AppController>,
) -> Result<Json<wire_clowder::ConnectedMintsResponse>> {
    Ok(Json(ctrl.clwdr_rest_client.get_betas().await?))
}

#[utoipa::path(
    get,
    path = ClowderClient::FOREIGN_OFFLINE_EP_V1,
    params(
        ("alpha_id" = String, Path, description = "The alpha public key")
    ),
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_foreign_offline(
    State(ctrl): State<AppController>,
    Path(alpha_id): Path<bitcoin::secp256k1::PublicKey>,
) -> Result<Json<wire_clowder::OfflineResponse>> {
    tracing::debug!("Requested /v1/foreign/offline/{alpha_id}");
    Ok(Json(ctrl.clwdr_rest_client.get_offline(alpha_id).await?))
}

#[utoipa::path(
    get,
    path = ClowderClient::FOREIGN_STATUS_EP_V1,
    params(
        ("alpha_id" = String, Path, description = "The alpha public key")
    ),
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_foreign_status(
    State(ctrl): State<AppController>,
    Path(alpha_id): Path<bitcoin::secp256k1::PublicKey>,
) -> Result<Json<wire_clowder::AlphaStateResponse>> {
    tracing::debug!("Requested /v1/foreign/status/{alpha_id}");
    Ok(Json(ctrl.clwdr_rest_client.get_status(alpha_id).await?))
}

#[utoipa::path(
    get,
    path = ClowderClient::FOREIGN_SUBSTITUTE_EP_V1,
    params(
        ("alpha_id" = String, Path, description = "The alpha public key")
    ),
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_foreign_substitute(
    State(ctrl): State<AppController>,
    Path(alpha_id): Path<bitcoin::secp256k1::PublicKey>,
) -> Result<Json<wire_clowder::ConnectedMintResponse>> {
    tracing::debug!("Requested /v1/foreign/substitute/{alpha_id}");
    Ok(Json(ctrl.clwdr_rest_client.get_substitute(alpha_id).await?))
}

#[utoipa::path(
    get,
    path = ClowderClient::FOREIGN_KEYSETS_EP_V1,
    params(
        ("alpha_id" = String, Path, description = "The alpha public key")
    ),
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_foreign_keysets(
    State(ctrl): State<AppController>,
    Path(alpha_id): Path<bitcoin::secp256k1::PublicKey>,
) -> Result<Json<cashu::KeysResponse>> {
    tracing::debug!("Requested /v1/foreign/keysets/{alpha_id}");
    Ok(Json(
        ctrl.clwdr_rest_client.get_active_keysets(&alpha_id).await?,
    ))
}

#[utoipa::path(
    post,
    path = ClowderClient::ONLINE_EXCHANGE_EP_V1,
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, request))]
pub async fn post_online_exchange(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_exchange::OnlineExchangeRequest>,
) -> Result<Json<wire_exchange::OnlineExchangeResponse>> {
    if request.exchange_path.len() < 3 {
        return Err(Error::Invalid(String::from(
            "minimum exchange path [alpha_pk, this_mint_pk, wallet_pk] not met",
        )));
    }
    let clowder_keys = ForeignKeyClientWithClowder {
        clwdr_cl: ctrl.clwdr_rest_client.clone(),
        pk: request.exchange_path[0],
    };
    let input_type =
        determine_input_type(&clowder_keys, request.proofs.iter().map(|p| p.keyset_id)).await?;
    // Clone what we need for the stream before consuming request
    let stream_proofs = request.proofs.clone();
    let stream_exchange_path = request.exchange_path.clone();
    let wire_exchange::OnlineExchangeRequest {
        proofs,
        exchange_path,
    } = request;
    let proofs = match input_type {
        InputType::Sat => {
            ctrl.treasury_client
                .sat_exchange_online(proofs, exchange_path)
                .await?
        }
        InputType::CrSat => {
            ctrl.treasury_client
                .crsat_exchange_online(proofs, exchange_path)
                .await?
        }
    };
    if let Err(e) = ctrl
        .clwdr_stream_client
        .mint_foreign_ecash(
            messages::MintForeignEcashRequest {
                proofs: stream_proofs,
                exchange_path: stream_exchange_path,
            },
            messages::MintForeignEcashResponse {
                proofs: proofs.clone(),
            },
        )
        .await { tracing::error!("Failed to post online exchange to clowder stream: {e}") };
    let response = wire_exchange::OnlineExchangeResponse { proofs };
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = ClowderClient::OFFLINE_EXCHANGE_EP_V1,
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, request))]
pub async fn post_offline_exchange(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_exchange::OfflineExchangeRequest>,
) -> Result<Json<wire_exchange::OfflineExchangeResponse>> {
    let origin = ctrl
        .clwdr_rest_client
        .post_fingerprints_origin(request.fingerprints.clone())
        .await?;
    let clowder_keys = ForeignKeyClientWithClowder {
        clwdr_cl: ctrl.clwdr_rest_client.clone(),
        pk: origin.node_id,
    };
    let input_type = determine_input_type(
        &clowder_keys,
        request.fingerprints.iter().map(|fp| fp.keyset_id),
    )
    .await?;
    // Clone what we need for the stream before consuming request
    let stream_fingerprints = request.fingerprints.clone();
    let stream_hashes = request.hashes.clone();
    let wire_exchange::OfflineExchangeRequest {
        fingerprints,
        hashes,
        wallet_pk,
    } = request;
    let response = match input_type {
        InputType::Sat => {
            ctrl.treasury_client
                .sat_exchange_offline_raw(fingerprints, hashes, wallet_pk)
                .await?
        }
        InputType::CrSat => {
            ctrl.treasury_client
                .crsat_exchange_offline_raw(fingerprints, hashes, wallet_pk)
                .await?
        }
    };
    let serialized = STANDARD
        .decode(&response.content)
        .map_err(|e| Error::InvalidInput(e.to_string()))?;
    let payload: bcr_common::wire::exchange::OfflineExchangePayload =
        borsh::from_slice(&serialized)?;

    if let Err(e) = ctrl
        .clwdr_stream_client
        .mint_offline_foreign_ecash(
            messages::MintForeignOfflineEcashRequest {
                fingerprints: stream_fingerprints,
                hashes: stream_hashes,
                wallet_pk,
            },
            messages::MintForeignOfflineEcashResponse {
                proofs: payload.proofs,
            },
        )
        .await { tracing::error!("Failed to post offline exchange to clowder stream: {e}") };
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = ClowderClient::LOCAL_COVERAGE_EP_V1,
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn get_coverage(
    State(ctrl): State<AppController>,
) -> Result<Json<wire_clowder::Coverage>> {
    tracing::debug!("Requested /v1/local/coverage");
    let supply = ctrl.clwdr_rest_client.get_mint_circulating_supply().await?;
    let collateral = ctrl.clwdr_rest_client.get_mint_collateral().await?;
    Ok(Json(wire_clowder::Coverage {
        debit_circulating_supply: supply.debit,
        credit_circulating_supply: supply.credit,
        onchain_collateral: collateral.onchain,
        ebill_collateral: collateral.ebill,
        eiou_collateral: collateral.eiou,
    }))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, request))]
pub async fn post_commit(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_swap::SwapCommitmentRequest>,
) -> Result<Json<wire_swap::SwapCommitmentResponse>> {
    let now = chrono::Utc::now();
    let (ys, secrets, expiry) = ctrl
        .commit_srv
        .commit(now, &request)
        .await?;

    // stream commitment to Clowder and get signed response
    let clowder_req = messages::SwapCommitmentRequest {
        content: request.content.clone(),
        wallet_key: request.wallet_key,
        wallet_signature: request.wallet_signature,
    };
    let clowder_resp = ctrl
        .clwdr_stream_client
        .swap_commitment(clowder_req)
        .await?;

    // store commitment with the Clowder-signed signature
    ctrl.commit_srv
        .store_commitment(ys, secrets, request.wallet_key, clowder_resp.commitment, expiry)
        .await?;

    let serialized = borsh::to_vec(&request)?;
    let content = STANDARD.encode(&serialized);

    let response = wire_swap::SwapCommitmentResponse {
        content,
        commitment: clowder_resp.commitment,
    };
    Ok(Json(response))
}

#[derive(Debug, Clone, Copy)]
enum InputType {
    CrSat,
    Sat,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
trait KeyClientT {
    // returns the currency unit for the given keyset id
    async fn currency(&self, keyset_id: cashu::Id) -> Result<cashu::CurrencyUnit>;
}

#[async_trait]
impl KeyClientT for CoreClient {
    async fn currency(&self, keyset_id: cashu::Id) -> Result<cashu::CurrencyUnit> {
        let cr_response = self.keyset_info(keyset_id).await?;
        Ok(cr_response.unit)
    }
}
pub struct ForeignKeyClientWithClowder {
    clwdr_cl: Arc<clwdr_client::ClowderRestClient>,
    pk: bitcoin::secp256k1::PublicKey,
}
#[async_trait]
impl KeyClientT for ForeignKeyClientWithClowder {
    async fn currency(&self, keyset_id: cashu::Id) -> Result<cashu::CurrencyUnit> {
        let keys = self.clwdr_cl.get_keyset(&self.pk, &keyset_id).await?;
        let Some(keyset) = keys.keysets.first() else {
            return Err(Error::Invalid(format!("keyset {keyset_id} not found")));
        };
        Ok(keyset.unit.clone())
    }
}

async fn determine_input_type(
    key_cl: &impl KeyClientT,
    inputs: impl std::iter::Iterator<Item = cashu::Id>,
) -> Result<InputType> {
    let unique_kids = inputs.collect::<HashSet<_>>();
    let requests: JoinAll<_> = unique_kids
        .into_iter()
        .map(|kid| key_cl.currency(kid))
        .collect();
    let responses: Vec<_> = requests.await.into_iter().collect::<Result<_>>()?;
    let all_sats = responses
        .iter()
        .all(|unit| *unit == cashu::CurrencyUnit::Sat);
    if all_sats {
        return Ok(InputType::Sat);
    }
    let crsat = cashu::CurrencyUnit::Custom(String::from("crsat"));
    let all_crsat = responses.iter().all(|unit| *unit == crsat);
    if all_crsat {
        return Ok(InputType::CrSat);
    }
    Err(Error::InvalidInput(String::from(
        "mixed credit/debit inputs not allowed",
    )))
}

async fn test_for_htlc(
    proofs: &[cashu::Proof],
    input_type: InputType,
    tcl: &TreasuryClient,
) -> Result<cashu::Amount> {
    let mut total = cashu::Amount::ZERO;
    for proof in proofs {
        if let Some(cashu::Witness::HTLCWitness(cashu::HTLCWitness { preimage, .. })) =
            &proof.witness
        {
            let amount = match input_type {
                InputType::CrSat => tcl.try_crsat_htlc(preimage.to_string()).await?,
                InputType::Sat => tcl.try_sat_htlc(preimage.to_string()).await?,
            };
            total += amount;
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_wdc_utils::{keys::test_utils as keys_test, signatures::test_utils as signatures_test};
    use mockall::predicate::*;

    #[tokio::test]
    async fn determine_input_type_sat() {
        let (_, sat_keyset) = keys_test::generate_random_keyset();
        let amounts = [cashu::Amount::from(4u64), cashu::Amount::from(4u64)];
        let inputs = [signatures_test::generate_proofs(&sat_keyset, &amounts[..1])[0].clone()];
        let mut client = MockKeyClientT::new();
        let sat_kid = sat_keyset.id;
        client
            .expect_currency()
            .times(1)
            .with(eq(sat_kid))
            .returning(|_| Ok(cashu::CurrencyUnit::Sat));
        let inputtype = determine_input_type(&client, inputs.iter().map(|p| p.keyset_id))
            .await
            .unwrap();
        assert!(matches!(inputtype, InputType::Sat));
    }

    #[tokio::test]
    async fn determine_input_type_crsat() {
        let (_, keyset) = keys_test::generate_random_keyset();
        let amounts = [cashu::Amount::from(4u64), cashu::Amount::from(8u64)];
        let inputs = signatures_test::generate_proofs(&keyset, &amounts);
        let mut client = MockKeyClientT::new();
        client
            .expect_currency()
            .times(1)
            .returning(move |_| Ok(cashu::CurrencyUnit::Custom(String::from("crsat"))));
        let inputtype = determine_input_type(&client, inputs.iter().map(|p| p.keyset_id))
            .await
            .unwrap();
        assert!(matches!(inputtype, InputType::CrSat));
    }
}
