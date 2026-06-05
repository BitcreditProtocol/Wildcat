// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::{
    cashu::{self, MintVersion},
    client::treasury::Client as TreasuryClient,
    wire::{
        clowder as wire_clowder,
        info::{VersionInfo, WildcatInfo},
        swap as wire_swap,
    },
};
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
        .map(|(k, v)| match v {
            Some(v) => format!("{k} = {v}"),
            None => format!("{k} = ?"),
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
    let wire_swap::SwapRequest {
        inputs,
        outputs,
        commitment,
        attestation,
    } = request;
    let htlc_unlocked = test_for_htlc(&inputs, &ctrl.treasury_client).await?;
    tracing::info!("HTLC unlocked in intermint exchange: {}", htlc_unlocked);
    let signatures = ctrl
        .core_client
        .swap(inputs, outputs, commitment, attestation)
        .await?;
    Ok(Json(wire_swap::SwapResponse { signatures }))
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
