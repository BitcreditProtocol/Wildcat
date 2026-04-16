// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::{
    cashu::{self, MintVersion},
    client::treasury::Client as TreasuryClient,
    wire::{
        clowder::{self as wire_clowder, messages},
        exchange as wire_exchange,
        info::{VersionInfo, WildcatInfo},
        swap as wire_swap,
    },
};
use bitcoin::base64::{engine::general_purpose::STANDARD, Engine};
// ----- local imports
use crate::{
    error::{Error, Result},
    AppController,
};

// ----- end imports

pub async fn health() -> Result<&'static str> {
    Ok("{ \"status\": \"OK\" }")
}

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

pub async fn post_swap(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_swap::SwapRequest>,
) -> Result<Json<wire_swap::SwapResponse>> {
    tracing::debug!("Requested /v1/swap");

    let wire_swap::SwapRequest {
        inputs,
        outputs,
        commitment,
    } = request;
    let htlc_unlocked = test_for_htlc(&inputs, &ctrl.treasury_client).await?;
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
    Ok(Json(wire_swap::SwapResponse { signatures }))
}

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
    // Clone what we need for the stream before consuming request
    let stream_proofs = request.proofs.clone();
    let stream_exchange_path = request.exchange_path.clone();
    let wire_exchange::OnlineExchangeRequest {
        proofs,
        exchange_path,
    } = request;
    let proofs = ctrl
        .treasury_client
        .exchange_online(proofs, exchange_path)
        .await?;
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
        .await
    {
        tracing::error!("Failed to post online exchange to clowder stream: {e}");
    }
    let response = wire_exchange::OnlineExchangeResponse { proofs };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, request))]
pub async fn post_offline_exchange(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_exchange::OfflineExchangeRequest>,
) -> Result<Json<wire_exchange::OfflineExchangeResponse>> {
    let _origin = ctrl
        .clwdr_rest_client
        .post_fingerprints_origin(request.fingerprints.clone())
        .await?;
    // Clone what we need for the stream before consuming request
    let stream_fingerprints = request.fingerprints.clone();
    let stream_hashes = request.hashes.clone();
    let wire_exchange::OfflineExchangeRequest {
        fingerprints,
        hashes,
        wallet_pk,
    } = request;
    let response = ctrl
        .treasury_client
        .exchange_offline_raw(fingerprints, hashes, wallet_pk)
        .await?;
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
        .await
    {
        tracing::error!("Failed to post offline exchange to clowder stream: {e}");
    }
    Ok(Json(response))
}

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

async fn test_for_htlc(proofs: &[cashu::Proof], tcl: &TreasuryClient) -> Result<cashu::Amount> {
    let mut total = cashu::Amount::ZERO;
    for proof in proofs {
        if let Some(cashu::Witness::HTLCWitness(cashu::HTLCWitness { preimage, .. })) =
            &proof.witness
        {
            let amount = tcl.try_htlc(preimage.to_string()).await?;
            total += amount;
        }
    }
    Ok(total)
}
