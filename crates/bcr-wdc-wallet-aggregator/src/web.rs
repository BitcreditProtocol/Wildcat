// ----- standard library imports
use std::collections::HashSet;
// ----- extra library imports
use async_trait::async_trait;
use axum::extract::{Json, Path, State};
use bcr_wdc_key_client::KeyClient;
use cashu::{
    nut00 as cdk00, nut01 as cdk01, nut02 as cdk02, nut03 as cdk03, nut06 as cdk06, nut07 as cdk07,
    nut09 as cdk09, MintVersion,
};
use cdk::wallet::MintConnector;
// ----- local imports
use crate::error::{Error, Result};
use crate::{built_info, AppController};

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
pub async fn get_mint_info(State(ctrl): State<AppController>) -> Result<Json<cdk06::MintInfo>> {
    tracing::debug!("Requested /v1/info");
    let network = ctrl.ebpp_client.network().await?;
    let info = ctrl.cdk_client.get_mint_info().await?;
    let mut long_description = format!(
        r#"[ebpp]
network = {network}
"#
    );
    if !built_info::PKG_VERSION_PRE.is_empty() {
        let ebill_core = built::util::parse_versions(&built_info::DEPENDENCIES)
            .find_map(|(n, v)| if n == "bcr-ebill-core" { Some(v) } else { None })
            .unwrap_or(built::semver::Version::new(0, 0, 0));
        let cdk_mintd = info
            .version
            .as_ref()
            .map(|v| v.version.clone())
            .unwrap_or(String::from("0.0.0"));
        long_description += &format!(
            r#"
[versions]
bcr-ebill-core = {ebill_core}
cdk-mintd = {cdk_mintd}"#,
        );
    }
    let version = MintVersion {
        name: String::from("wildcat"),
        version: String::from(built_info::PKG_VERSION),
    };
    let info = info
        .name("bcr-wdc")
        .description("Wildcat One")
        .long_description(long_description)
        .version(version);
    Ok(Json(info))
}

#[utoipa::path(
    get,
    path = "/v1/keys",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_mint_keys(State(ctrl): State<AppController>) -> Result<Json<cdk01::KeysResponse>> {
    tracing::debug!("Requested /v1/keys");

    let mut keys = ctrl.cdk_client.get_mint_keys().await?;
    let mut bcr_keys = ctrl.keys_client.list_keys().await.unwrap_or_default();
    keys.append(&mut bcr_keys);
    let response = cdk01::KeysResponse { keysets: keys };
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
) -> Result<Json<cdk02::KeysetResponse>> {
    tracing::debug!("Requested /v1/keysets");

    let mut infos = ctrl.cdk_client.get_mint_keysets().await?;
    let mut bcr_infos = ctrl
        .keys_client
        .list_keyset_info()
        .await
        .unwrap_or_default();
    infos.keysets.append(&mut bcr_infos);
    Ok(Json(infos))
}

#[utoipa::path(
    get,
    path = "/v1/keys/{kid}",
    params(
        ("kid" = cdk02::Id, Path, description = "The keyset id")
    ),
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
        (status = 404, description = "Keyset not found"),
    )
)]
pub async fn get_mint_keyset(
    State(ctrl): State<AppController>,
    Path(kid): Path<cdk02::Id>,
) -> Result<Json<cdk01::KeysResponse>> {
    tracing::debug!("Requested /v1/keys/{}", kid);

    let bcr_response = ctrl.keys_client.keys(kid).await;
    if let Ok(keys) = bcr_response {
        let response = cdk01::KeysResponse {
            keysets: vec![keys],
        };
        return Ok(Json(response));
    }
    let keys = ctrl.cdk_client.get_mint_keyset(kid).await?;
    let response = cdk01::KeysResponse {
        keysets: vec![keys],
    };
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/v1/keysets/{kid}",
    params(
        ("kid" = cdk02::Id, Path, description = "The keyset id")
    ),
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
        (status = 404, description = "Keyset not found"),
    )
)]
pub async fn get_keyset_info(
    State(ctrl): State<AppController>,
    Path(kid): Path<cdk02::Id>,
) -> Result<Json<cdk02::KeySetInfo>> {
    tracing::debug!("Requested /v1/keysets/{}", kid);

    if let Ok(info) = ctrl.keys_client.keyset_info(kid).await {
        Ok(Json(info))
    } else {
        let keysets = ctrl.cdk_client.get_mint_keysets().await?.keysets;

        for active_keyset in keysets {
            if active_keyset.id == kid {
                return Ok(Json(active_keyset));
            }
        }
        // if there are no keys, then it doesn't exist, otherwise its inactive
        let keys = ctrl.cdk_client.get_mint_keyset(kid).await?;
        Ok(Json(cdk02::KeySetInfo {
            id: kid,
            active: false,
            unit: keys.unit,
            final_expiry: keys.final_expiry,
            // Fee doesn't matter as we cannot swap into it
            // we can only swap into a different active keyset
            input_fee_ppk: 0,
        }))
    }
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
    Json(request): Json<cdk03::SwapRequest>,
) -> Result<Json<cdk03::SwapResponse>> {
    tracing::debug!("Requested /v1/swap");

    let swap_type = determine_swap_type(
        &ctrl.keys_client,
        request.inputs().as_slice(),
        request.outputs().as_slice(),
    )
    .await?;
    let signatures = match swap_type {
        SwapType::CrSat2CrSat => {
            ctrl.swap_client
                .swap(request.inputs().to_vec(), request.outputs().to_vec())
                .await?
        }
        SwapType::CrSat2Sat => {
            ctrl.treasury_client
                .redeem(request.inputs().to_vec(), request.outputs().to_vec())
                .await?
        }
        SwapType::Sat2Sat => ctrl
            .cdk_client
            .post_swap(request)
            .await
            .map(|resp| resp.signatures)?,
    };

    let response = cdk03::SwapResponse { signatures };
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
    Json(request): Json<cdk07::CheckStateRequest>,
) -> Result<Json<cdk07::CheckStateResponse>> {
    tracing::debug!("Requested /v1/checkstate");

    let n = request.ys.len();
    let credit_states = ctrl.swap_client.check_state(request.ys.clone()).await?;
    let debit_states = ctrl.cdk_client.post_check_state(request).await?.states;
    // TODO ensure the order and length are the same as the input, which should always be the case anyway
    if debit_states.len() != n || credit_states.len() != n {
        return Err(Error::NotYet(
            "Unhandled credit and debit length mismatch".into(),
        ));
    }

    let mut merged = Vec::new();
    for (debit, credit) in debit_states.iter().zip(credit_states.iter()) {
        if debit.state != cashu::nut07::State::Unspent
            && credit.state != cashu::nut07::State::Unspent
        {
            return Err(Error::NotYet("Unhandled credit and debit are spent".into()));
        }
        if debit.state != cashu::nut07::State::Unspent {
            merged.push(debit.clone());
        } else {
            merged.push(credit.clone());
        }
    }
    Ok(Json(cdk07::CheckStateResponse { states: merged }))
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
    Json(request): Json<cdk09::RestoreRequest>,
) -> Result<Json<cdk09::RestoreResponse>> {
    tracing::debug!("Requested /v1/restore");

    let outputs = request.outputs.clone();
    let crsat_signatures = ctrl.keys_client.restore(outputs.clone()).await?;
    let restore_resp = ctrl.cdk_client.post_restore(request).await?;
    let sat_signatures = restore_resp
        .outputs
        .into_iter()
        .zip(restore_resp.signatures.into_iter())
        .collect::<Vec<_>>();

    let mut response = cdk09::RestoreResponse {
        outputs: Default::default(),
        signatures: Default::default(),
        promises: Default::default(),
    };
    // we assume that both sat_signatures and crsat_signatures are ordered
    // according to the order of request.outputs
    // as described in NUT09
    let mut crsat_c = 0;
    let mut sat_c = 0;
    for blind in outputs {
        if let Some(element) = crsat_signatures.get(crsat_c) {
            if blind.blinded_secret == element.0.blinded_secret {
                response.outputs.push(element.0.clone());
                response.signatures.push(element.1.clone());
                crsat_c += 1;
            }
        }
        if let Some(element) = sat_signatures.get(sat_c) {
            if blind.blinded_secret == element.0.blinded_secret {
                response.outputs.push(element.0.clone());
                response.signatures.push(element.1.clone());
                sat_c += 1;
            }
        }
    }
    Ok(Json(response))
}

#[allow(clippy::enum_variant_names)]
enum SwapType {
    CrSat2CrSat,
    Sat2Sat,
    CrSat2Sat,
}

/// if any keyset ID among the inputs is not found in crsat-key-service, then the swap can only be
/// a sat2sat
/// once proved that all inputs are found in crsat-key-service,
/// if any keyset ID among the outputs is not found in crsat-key-service, then the swap can only be
/// a crsat2sat
/// once proved that all outputs are found in crsat-key-service,
/// then it's definitely a crsat2crsat swap
/// it's not a responsibility of this service to deal with the case of mixed inputs/outputs
#[cfg_attr(test, mockall::automock)]
#[async_trait]
trait KeyClientT {
    async fn keyset_info(
        &self,
        keyset_id: cdk02::Id,
    ) -> bcr_wdc_key_client::Result<cdk02::KeySetInfo>;
}
#[async_trait]
impl KeyClientT for KeyClient {
    async fn keyset_info(
        &self,
        keyset_id: cdk02::Id,
    ) -> bcr_wdc_key_client::Result<cdk02::KeySetInfo> {
        self.keyset_info(keyset_id).await
    }
}

async fn determine_swap_type(
    key_cl: &impl KeyClientT,
    inputs: &[cdk00::Proof],
    outputs: &[cdk00::BlindedMessage],
) -> Result<SwapType> {
    let input_kids = inputs.iter().map(|p| p.keyset_id).collect::<HashSet<_>>();
    for kid in input_kids.into_iter() {
        let response = key_cl.keyset_info(kid).await;
        match response {
            Err(bcr_wdc_key_client::Error::ResourceNotFound(_)) => return Ok(SwapType::Sat2Sat),
            Err(e) => return Err(Error::Keys(e)),
            Ok(_) => {}
        }
    }
    let output_kids = outputs.iter().map(|b| b.keyset_id).collect::<HashSet<_>>();
    for kid in output_kids.into_iter() {
        let response = key_cl.keyset_info(kid).await;
        match response {
            Err(bcr_wdc_key_client::Error::ResourceNotFound(_)) => return Ok(SwapType::CrSat2Sat),
            Err(e) => return Err(Error::Keys(e)),
            Ok(_) => {}
        }
    }

    Ok(SwapType::CrSat2CrSat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_wdc_utils::{keys::test_utils as keys_test, signatures::test_utils as signatures_test};
    use mockall::predicate::*;

    #[tokio::test]
    async fn determine_swap_type_sat2sat() {
        let (_, sat_keyset) = keys_test::generate_random_keyset();
        let (crsat_info, crsat_keyset) = keys_test::generate_random_keyset();
        let amounts = [cashu::Amount::from(4u64), cashu::Amount::from(4u64)];
        let inputs = [
            signatures_test::generate_proofs(&crsat_keyset, &amounts[1..])[0].clone(),
            signatures_test::generate_proofs(&sat_keyset, &amounts[..1])[0].clone(),
        ];
        let outputs: Vec<cdk00::BlindedMessage> =
            signatures_test::generate_blinds(sat_keyset.id, &amounts)
                .into_iter()
                .map(|(b, _, _)| b)
                .collect();
        let mut client = MockKeyClientT::new();
        let crsat_kid = crsat_keyset.id;
        client
            .expect_keyset_info()
            .times(1)
            .with(eq(crsat_kid))
            .returning(move |_| Ok(cashu::KeySetInfo::from(crsat_info.clone())));
        let sat_kid = sat_keyset.id;
        client
            .expect_keyset_info()
            .times(1)
            .with(eq(sat_kid))
            .returning(|kid| Err(bcr_wdc_key_client::Error::ResourceNotFound(kid)));

        let swaptype = determine_swap_type(&client, &inputs, &outputs)
            .await
            .unwrap();
        assert!(matches!(swaptype, SwapType::Sat2Sat));
    }

    #[tokio::test]
    async fn determine_swap_type_crsat2crsat() {
        let (info, keyset) = keys_test::generate_random_keyset();
        let amounts = [cashu::Amount::from(4u64), cashu::Amount::from(8u64)];
        let inputs = signatures_test::generate_proofs(&keyset, &amounts);
        let outputs: Vec<cdk00::BlindedMessage> =
            signatures_test::generate_blinds(keyset.id, &amounts)
                .into_iter()
                .map(|(b, _, _)| b)
                .collect();
        let mut client = MockKeyClientT::new();
        client
            .expect_keyset_info()
            .times(2)
            .returning(move |_| Ok(cashu::KeySetInfo::from(info.clone())));
        let swaptype = determine_swap_type(&client, &inputs, &outputs)
            .await
            .unwrap();
        assert!(matches!(swaptype, SwapType::CrSat2CrSat));
    }

    #[tokio::test]
    async fn determine_swap_type_crsat2sat() {
        let (_, sat_keyset) = keys_test::generate_random_keyset();
        let (crsat_info, crsat_keyset) = keys_test::generate_random_keyset();
        let amounts = [cashu::Amount::from(4u64), cashu::Amount::from(4u64)];
        let inputs = signatures_test::generate_proofs(&crsat_keyset, &amounts);
        let outputs: Vec<cdk00::BlindedMessage> =
            signatures_test::generate_blinds(sat_keyset.id, &amounts)
                .into_iter()
                .map(|(b, _, _)| b)
                .collect();
        let mut client = MockKeyClientT::new();
        let crsat_kid = crsat_keyset.id;
        client
            .expect_keyset_info()
            .times(1)
            .with(eq(crsat_kid))
            .returning(move |_| Ok(cashu::KeySetInfo::from(crsat_info.clone())));
        let sat_kid = sat_keyset.id;
        client
            .expect_keyset_info()
            .times(1)
            .with(eq(sat_kid))
            .returning(|kid| Err(bcr_wdc_key_client::Error::ResourceNotFound(kid)));

        let swaptype = determine_swap_type(&client, &inputs, &outputs)
            .await
            .unwrap();
        assert!(matches!(swaptype, SwapType::CrSat2Sat));
    }
}
