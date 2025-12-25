// ----- standard library imports
use std::{collections::HashSet, sync::Arc};
// ----- extra library imports
use async_trait::async_trait;
use axum::extract::{Json, Path, State};
use bcr_common::{
    client::keys::{Client as KeysClient, Error as KeysError},
    wire::{exchange as wire_exchange, swap as wire_swap},
};
use bcr_wdc_treasury_client::TreasuryClient;
use cashu::MintVersion;
use cdk::wallet::MintConnector;
use futures::future::JoinAll;
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    {built_info, AppController},
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
    let network = ctrl.ebpp_client.network().await?;
    let info = ctrl.cdk_client.get_mint_info().await?;
    let mut long_description = format!(
        r#"[ebpp]
network = {network}
"#
    );
    if !built_info::PKG_VERSION_PRE.is_empty() {
        let build_time = built::util::strptime(built_info::BUILT_TIME_UTC);
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
build-time = {build_time}
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
pub async fn get_mint_keys(State(ctrl): State<AppController>) -> Result<Json<cashu::KeysResponse>> {
    tracing::debug!("Requested /v1/keys");

    let mut keys = ctrl.cdk_client.get_mint_keys().await?;
    let mut bcr_keys = ctrl.keys_client.list_keys().await.unwrap_or_default();
    keys.append(&mut bcr_keys);
    let response = cashu::KeysResponse { keysets: keys };
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

    let bcr_response = ctrl.keys_client.keys(kid).await;
    if let Ok(keys) = bcr_response {
        let response = cashu::KeysResponse {
            keysets: vec![keys],
        };
        return Ok(Json(response));
    }
    let keys = ctrl.cdk_client.get_mint_keyset(kid).await?;
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
        Ok(Json(cashu::KeySetInfo {
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
    Json(request): Json<cashu::SwapRequest>,
) -> Result<Json<cashu::SwapResponse>> {
    tracing::debug!("Requested /v1/swap");

    let now = chrono::Utc::now();
    let is_ok = ctrl.commit_srv.check_swap(now, request.clone()).await?;
    if !is_ok {
        return Err(Error::InvalidInput(String::from(
            "Swap request rejected due to commitment",
        )));
    }
    let keyscl = KeysClients {
        credit: ctrl.keys_client.clone(),
        debit: ctrl.cdk_client.clone(),
    };
    let input_type = determine_input_type(&keyscl, request.inputs()).await?;
    let output_type = determine_output_type(&keyscl, request.outputs()).await?;
    let swap_type = io_to_swap(input_type, output_type)?;
    let proofs = request.inputs().clone();
    let blinded_messages = request.outputs();
    let htlc_unlocked = test_for_htlc(&proofs, input_type, &ctrl.treasury_client).await?;
    tracing::info!("HTLC unlocked in intermint exchange: {}", htlc_unlocked);

    let signatures = match swap_type {
        SwapType::CrSat2CrSat => {
            ctrl.swap_client
                .swap(proofs.clone(), blinded_messages.clone())
                .await?
        }
        SwapType::CrSat2Sat => {
            ctrl.treasury_client
                .redeem(proofs.clone(), blinded_messages.clone())
                .await?
        }
        SwapType::Sat2Sat => ctrl
            .cdk_client
            .post_swap(request)
            .await
            .map(|resp| resp.signatures)?,
    };

    if let Some(clwdr_client) = ctrl.clwdr_stream_client {
        clwdr_client.mint_swap(proofs, signatures.clone()).await?;
    }

    let response = cashu::SwapResponse { signatures };

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
    Ok(Json(cashu::CheckStateResponse { states: merged }))
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

    let outputs = request.outputs.clone();
    let crsat_signatures = ctrl.keys_client.restore(outputs.clone()).await?;
    let restore_resp = ctrl.cdk_client.post_restore(request).await?;
    let sat_signatures = restore_resp
        .outputs
        .into_iter()
        .zip(restore_resp.signatures.into_iter())
        .collect::<Vec<_>>();

    let mut response = cashu::RestoreResponse {
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

#[utoipa::path(
    get,
    path = "/v1/id",
    responses (
        (status = 200, description = "Successful response", content_type = "application/json"),
    )
)]
pub async fn get_clowder_id(
    State(ctrl): State<AppController>,
) -> Result<Json<clwdr_client::model::PublicKeyResponse>> {
    let clowder_client = ctrl.clwdr_rest_client.ok_or(Error::ClowderClientNoInit)?;

    Ok(Json(clowder_client.get_id().await?))
}

// TODO, add Utoipa ToSchema for PathRequest
pub async fn post_clowder_path(
    State(ctrl): State<AppController>,
    Json(request): Json<clwdr_client::model::PathRequest>,
) -> Result<Json<clwdr_client::model::ConnectedMintsResponse>> {
    let clowder_client = ctrl.clwdr_rest_client.ok_or(Error::ClowderClientNoInit)?;

    Ok(Json(
        clowder_client.post_path(request.origin_mint_url).await?,
    ))
}

pub async fn get_clowder_betas(
    State(ctrl): State<AppController>,
) -> Result<Json<clwdr_client::model::ConnectedMintsResponse>> {
    let clowder_client = ctrl.clwdr_rest_client.ok_or(Error::ClowderClientNoInit)?;

    Ok(Json(clowder_client.get_betas().await?))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, request))]
pub async fn post_exchange(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_exchange::OnlineExchangeRequest>,
) -> Result<Json<wire_exchange::OnlineExchangeResponse>> {
    if request.exchange_path.len() < 3 {
        return Err(Error::Invalid(String::from(
            "minimum exchange path [alpha_pk, this_mint_pk, wallet_pk] not met",
        )));
    }
    let Some(rest_clwdr) = ctrl.clwdr_rest_client.as_ref() else {
        return Err(Error::ClowderClientNoInit);
    };
    let clowder_keys = ForeignKeyClientWithClowder {
        clwdr_cl: rest_clwdr.clone(),
        pk: request.exchange_path[0],
    };
    let input_type = determine_input_type(&clowder_keys, &request.proofs).await?;
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
    let response = wire_exchange::OnlineExchangeResponse { proofs };
    Ok(Json(response))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn melt_quote_onchain(
    State(ctrl): State<AppController>,
    Json(request): Json<bcr_common::wire::melt::MeltQuoteOnchainRequest>,
) -> Result<Json<cashu::nuts::MeltQuoteBolt11Response<String>>> {
    let expiry = chrono::Utc::now().timestamp() + 86400;

    let quote_id = ctrl
        .treasury_client
        .store_onchain_melt(request.clone())
        .await
        .map_err(|e| Error::Treasury(e))?;

    let id = quote_id.to_string();

    Ok(Json(cashu::nuts::MeltQuoteBolt11Response {
        quote: id,
        fee_reserve: cashu::Amount::ZERO,
        paid: Some(false),
        payment_preimage: None,
        change: None,
        amount: request.request.amount,
        unit: Some(request.unit),
        request: None,
        state: cashu::nuts::MeltQuoteState::Unpaid,
        expiry: expiry as u64,
    }))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn melt_onchain(
    State(ctrl): State<AppController>,
    Json(request): Json<cashu::MeltRequest<String>>,
) -> Result<Json<()>> {
    tracing::info!("Melt onchain request received");

    let quote_id_str = request.quote_id();
    let quote_id = Uuid::parse_str(quote_id_str)
        .map_err(|_| Error::InvalidInput(String::from("Invalid quote ID")))?;

    let onchain_request = ctrl
        .treasury_client
        .load_onchain_melt(quote_id)
        .await
        .map_err(|e| Error::Treasury(e))?;

    let inputs = request.inputs();
    if inputs.is_empty() {
        return Err(Error::InvalidInput(String::from("No inputs")));
    }

    let total_blinds = request.output_amount().unwrap_or(cashu::Amount::ZERO);

    let total_proofs = request
        .inputs_amount()
        .map_err(|_| Error::InvalidInput(String::from("No amount for inputs")))?;

    tracing::info!(
        "Melt request Total proofs to burn {} change {}",
        total_proofs,
        total_blinds
    );

    if total_proofs != onchain_request.request.amount {
        tracing::warn!("Total proofs amount does not match the quoted amount");
        return Err(Error::InvalidInput(String::from(
            "Requested amount mismatch",
        )));
    }

    if let Some(clowder) = ctrl.clwdr_stream_client {
        clowder.melt_onchain(request, onchain_request).await?;
    }

    Ok(Json(()))
}

#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl, request))]
pub async fn post_commit(
    State(ctrl): State<AppController>,
    Json(request): Json<wire_swap::CommitmentRequest>,
) -> Result<Json<wire_swap::CommitmentResponse>> {
    let now = chrono::Utc::now();
    let response = ctrl
        .commit_srv
        .commit(
            now,
            request,
            &ctrl.swap_client,
            &ctrl.keys_client,
            &ctrl.cdk_client,
        )
        .await?;
    Ok(Json(response))
}

#[derive(Debug, Clone, Copy)]
enum InputType {
    CrSat,
    Sat,
}
enum OutputType {
    CrSat,
    Sat,
}

#[allow(clippy::enum_variant_names)]
enum SwapType {
    CrSat2CrSat,
    Sat2Sat,
    CrSat2Sat,
}

fn io_to_swap(input: InputType, output: OutputType) -> Result<SwapType> {
    match (input, output) {
        (InputType::CrSat, OutputType::CrSat) => Ok(SwapType::CrSat2CrSat),
        (InputType::Sat, OutputType::Sat) => Ok(SwapType::Sat2Sat),
        (InputType::CrSat, OutputType::Sat) => Ok(SwapType::CrSat2Sat),
        (InputType::Sat, OutputType::CrSat) => {
            Err(Error::Invalid(String::from("swap not allowed")))
        }
    }
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
trait KeyClientT {
    // returns the currency unit for the given keyset id
    async fn currency(&self, keyset_id: cashu::Id) -> Result<cashu::CurrencyUnit>;
}

struct KeysClients {
    credit: KeysClient,
    debit: cdk::wallet::HttpClient,
}
#[async_trait]
impl KeyClientT for KeysClients {
    async fn currency(&self, keyset_id: cashu::Id) -> Result<cashu::CurrencyUnit> {
        let cr_response = self.credit.keyset_info(keyset_id).await;
        match cr_response {
            Ok(info) => return Ok(info.unit),
            Err(KeysError::KeysetIdNotFound(_)) => {}
            Err(e) => return Err(Error::Keys(e)),
        }
        let db_response = self.debit.get_mint_keyset(keyset_id).await;
        match db_response {
            Ok(info) => return Ok(info.unit),
            Err(e) => return Err(Error::Cdk(e)),
        }
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
    inputs: &[cashu::Proof],
) -> Result<InputType> {
    let unique_kids = inputs.iter().map(|p| p.keyset_id).collect::<HashSet<_>>();
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

async fn determine_output_type(
    key_cl: &impl KeyClientT,
    outputs: &[cashu::BlindedMessage],
) -> Result<OutputType> {
    let unique_kids = outputs.iter().map(|b| b.keyset_id).collect::<HashSet<_>>();
    let requests: JoinAll<_> = unique_kids
        .into_iter()
        .map(|kid| key_cl.currency(kid))
        .collect();
    let responses: Vec<_> = requests.await.into_iter().collect::<Result<_>>()?;
    let all_sats = responses
        .iter()
        .all(|unit| *unit == cashu::CurrencyUnit::Sat);
    if all_sats {
        return Ok(OutputType::Sat);
    }
    let crsat = cashu::CurrencyUnit::Custom(String::from("crsat"));
    let all_crsat = responses.iter().all(|unit| *unit == crsat);
    if all_crsat {
        return Ok(OutputType::CrSat);
    }
    Err(Error::InvalidInput(String::from(
        "mixed credit/debit outputs not allowed",
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
        let inputtype = determine_input_type(&client, &inputs).await.unwrap();
        assert!(matches!(inputtype, InputType::Sat));
    }

    #[tokio::test]
    async fn determine_output_type_sat() {
        let (_, sat_keyset) = keys_test::generate_random_keyset();
        let amounts = [cashu::Amount::from(4u64), cashu::Amount::from(4u64)];
        let outputs: Vec<cashu::BlindedMessage> =
            signatures_test::generate_blinds(sat_keyset.id, &amounts)
                .into_iter()
                .map(|(b, _, _)| b)
                .collect();
        let mut client = MockKeyClientT::new();
        let sat_kid = sat_keyset.id;
        client
            .expect_currency()
            .times(1)
            .with(eq(sat_kid))
            .returning(|_| Ok(cashu::CurrencyUnit::Sat));

        let outputtype = determine_output_type(&client, &outputs).await.unwrap();
        assert!(matches!(outputtype, OutputType::Sat));
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
        let inputtype = determine_input_type(&client, &inputs).await.unwrap();
        assert!(matches!(inputtype, InputType::CrSat));
    }

    #[tokio::test]
    async fn determine_swap_type_crsat2crsat() {
        let (_, keyset) = keys_test::generate_random_keyset();
        let amounts = [cashu::Amount::from(4u64), cashu::Amount::from(8u64)];
        let outputs: Vec<cashu::BlindedMessage> =
            signatures_test::generate_blinds(keyset.id, &amounts)
                .into_iter()
                .map(|(b, _, _)| b)
                .collect();
        let mut client = MockKeyClientT::new();
        client
            .expect_currency()
            .times(1)
            .returning(move |_| Ok(cashu::CurrencyUnit::Custom(String::from("crsat"))));
        let outputtype = determine_output_type(&client, &outputs).await.unwrap();
        assert!(matches!(outputtype, OutputType::CrSat));
    }
}
