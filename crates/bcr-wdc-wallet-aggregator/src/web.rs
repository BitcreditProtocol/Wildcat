// ----- standard library imports
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::{
    cashu::{self, MintVersion},
    client::treasury::Client as TreasuryClient,
    wire::swap as wire_swap,
};
// ----- local imports
use crate::{error::Result, AppController};

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

pub async fn post_swap(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_swap::SwapRequest>,
) -> Result<Json<wire_swap::SwapResponse>> {
    let wire_swap::SwapRequest {
        inputs,
        outputs,
        commitment,
    } = request;
    let htlc_unlocked = test_for_htlc(&inputs, &ctrl.treasury_client).await?;
    tracing::info!("HTLC unlocked in intermint exchange: {}", htlc_unlocked);
    let signatures = ctrl.core_client.swap(inputs, outputs, commitment).await?;
    Ok(Json(wire_swap::SwapResponse { signatures }))
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
