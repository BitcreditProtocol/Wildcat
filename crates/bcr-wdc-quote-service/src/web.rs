// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, Path, State};
use bcr_common::{
    core::signature::{deserialize_borsh_msg, schnorr_verify_b64},
    wire::quotes as wire_quotes,
};
// ----- local imports
use crate::{error::Result, quotes, service::Service};

// ----- end imports

///--------------------------- Enquire mint quote
pub async fn enquire_quote(
    State(ctrl): State<Arc<Service>>,
    Json(signed_request): Json<wire_quotes::SignedEnquireRequest>,
) -> Result<Json<wire_quotes::EnquireReply>> {
    tracing::debug!("Received mint quote request for bill",);

    let payload: wire_quotes::EnquireRequest = deserialize_borsh_msg(&signed_request.content)?;
    let bill_info = ctrl
        .validate_and_decrypt_shared_bill(&payload.content)
        .await?;
    // after validating bill, validate req using the calculated holder
    let holder = bill_info.endorsees.last().unwrap_or(&bill_info.payee);
    schnorr_verify_b64(
        &signed_request.content,
        &signed_request.signature,
        &holder.node_id().pub_key().x_only_public_key().0,
    )?;
    let bill = quotes::convert_to_billinfo(bill_info, payload.content)?;
    let id = ctrl
        .enquire(bill, payload.minting_pubkey, chrono::Utc::now())
        .await?;
    Ok(Json(wire_quotes::EnquireReply { id }))
}

/// Holder-authorized, AI-permitted replacement of one recently denied quote.
pub async fn reissue_enquire_quote(
    State(ctrl): State<Arc<Service>>,
    Json(signed_request): Json<wire_quotes::SignedReissueEnquireRequestV1>,
) -> Result<Json<wire_quotes::EnquireReply>> {
    let payload: wire_quotes::ReissueEnquireRequestV1 =
        deserialize_borsh_msg(&signed_request.content)?;
    if payload.schema_version != wire_quotes::REISSUE_ENQUIRE_SCHEMA_VERSION
        || payload.action != wire_quotes::REISSUE_ENQUIRE_ACTION
        || payload.signed_permit.permit.action != payload.action
    {
        return Err(crate::error::Error::CreditQuoteReissueInvalid);
    }
    let bill_info = ctrl
        .validate_and_decrypt_shared_bill(&payload.content)
        .await?;
    let holder = bill_info.endorsees.last().unwrap_or(&bill_info.payee);
    schnorr_verify_b64(
        &signed_request.content,
        &signed_request.signature,
        &holder.node_id().pub_key().x_only_public_key().0,
    )?;
    let bill = quotes::convert_to_billinfo(bill_info, payload.content)?;
    let id = ctrl
        .reissue_enquire(
            bill,
            payload.minting_pubkey,
            payload.signed_permit,
            chrono::Utc::now(),
        )
        .await?;
    Ok(Json(wire_quotes::EnquireReply { id }))
}

/// --------------------------- Look up quote
fn convert_to_enquire_reply(quote: quotes::Quote) -> wire_quotes::StatusReply {
    match quote.status {
        quotes::Status::Pending { .. } => wire_quotes::StatusReply::Pending,
        quotes::Status::Canceled { tstamp } => wire_quotes::StatusReply::Canceled { tstamp },
        quotes::Status::Denied { tstamp } => wire_quotes::StatusReply::Denied { tstamp },
        quotes::Status::Offered {
            keyset_id,
            ttl,
            discounted,
            wallet_pubkey,
        } => wire_quotes::StatusReply::Offered {
            keyset_id,
            expiration_date: ttl,
            discounted,
            wallet_pubkey,
        },
        quotes::Status::OfferExpired { tstamp, discounted } => {
            wire_quotes::StatusReply::OfferExpired { tstamp, discounted }
        }
        quotes::Status::Rejected { tstamp, discounted } => {
            wire_quotes::StatusReply::Rejected { tstamp, discounted }
        }
        quotes::Status::Accepted {
            keyset_id,
            discounted,
            wallet_pubkey,
        } => wire_quotes::StatusReply::Accepted {
            keyset_id,
            discounted,
            wallet_pubkey,
        },
        quotes::Status::MintingEnabled {
            keyset_id,
            wallet_pubkey,
            discounted,
            ..
        } => wire_quotes::StatusReply::MintingEnabled {
            keyset_id,
            discounted,
            wallet_pubkey,
            minted_amount: bcr_common::cashu::Amount::ZERO,
        },
        quotes::Status::FailedEbillValidation {
            keyset_id,
            discounted,
            wallet_pubkey,
        } => wire_quotes::StatusReply::FailedEbillValidation {
            keyset_id,
            discounted,
            wallet_pubkey,
        },
    }
}

pub async fn lookup_quote(
    State(ctrl): State<Arc<Service>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<wire_quotes::StatusReply>> {
    tracing::debug!("Received mint quote lookup request for id: {}", id);

    let now = chrono::Utc::now();
    let quote = ctrl.lookup(id, now).await?;
    Ok(Json(convert_to_enquire_reply(quote)))
}

/// --------------------------- Resolve quote offer
pub async fn resolve_offer(
    State(ctrl): State<Arc<Service>>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<wire_quotes::ResolveOffer>,
) -> Result<()> {
    tracing::debug!("Received mint quote resolve request for id: {}", id);

    let now = chrono::Utc::now();
    match req {
        wire_quotes::ResolveOffer::Reject => ctrl.reject(id, now).await?,
        wire_quotes::ResolveOffer::Accept => ctrl.accept(id, now).await?,
    }
    Ok(())
}

/// --------------------------- Cancel quote inquiry
pub async fn cancel(
    State(ctrl): State<Arc<Service>>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<wire_quotes::StatusReply>> {
    tracing::debug!("Received mint quote cancel request for id: {}", id);

    let now = chrono::Utc::now();
    ctrl.cancel(id, now).await?;
    let quote = ctrl.lookup(id, now).await?;
    let reply = convert_to_enquire_reply(quote);
    Ok(Json(reply))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_common::{
        cashu,
        core::{generate_random_keypair, signature::serialize_n_schnorr_sign_borsh_msg},
        wire::{bill::BillParticipant, test_utils as wire_tests},
    };
    use std::str::FromStr;

    #[tokio::test]
    async fn reissue_rejects_a_request_not_signed_by_the_current_holder() {
        let (holder_key, holder) = wire_tests::random_identity_public_data();
        let (_, drawee) = wire_tests::random_identity_public_data();
        let (_, drawer) = wire_tests::random_identity_public_data();
        let bill_id = bcr_common::core_tests::random_bill_id();
        let shared = wire_quotes::SharedBill {
            bill_id: bill_id.clone(),
            data: String::from("encrypted-bill"),
            file_urls: vec![],
            hash: String::from("bill-hash"),
            signature: String::from("bill-signature"),
            receiver: bitcoin::PublicKey::from_str(
                "026423b7d36d05b8d50a89a1b4ef2a06c88bcd2c5e650f25e122fa682d3b39686c",
            )
            .unwrap(),
        };
        let wire_bill = wire_quotes::BillInfo {
            id: bill_id,
            drawee,
            drawer,
            payee: BillParticipant::Ident(holder),
            endorsees: vec![],
            sum: 8_000_000,
            maturity_date: chrono::Utc::now().date_naive() + chrono::Duration::days(30),
            file_urls: vec![],
        };
        let now = chrono::Utc::now();
        let previous = quotes::Quote::new(
            quotes::BillInfo::random(),
            cashu::PublicKey::from(holder_key.public_key()),
            now,
            quotes::test_credit_program_binding(),
        );
        let payload = wire_quotes::ReissueEnquireRequestV1 {
            schema_version: wire_quotes::REISSUE_ENQUIRE_SCHEMA_VERSION.to_owned(),
            action: wire_quotes::REISSUE_ENQUIRE_ACTION.to_owned(),
            content: shared,
            minting_pubkey: cashu::PublicKey::from(holder_key.public_key()),
            signed_permit: crate::authorization::tests::signed_reissue_for(
                &previous,
                uuid::Uuid::new_v4(),
                now,
                now + chrono::Duration::hours(1),
            ),
        };
        let attacker = generate_random_keypair();
        let (content, signature) = serialize_n_schnorr_sign_borsh_msg(&payload, &attacker).unwrap();
        let mut wdc = crate::service::MockWdcClient::new();
        wdc.expect_validate_and_decrypt_shared_bill()
            .return_once(move |_| Ok(wire_bill));
        let service = Arc::new(Service {
            wdc_client: Box::new(wdc),
            quotes: Box::new(crate::persistence::MockRepository::new()),
            mint_url: cashu::MintUrl::from_str("http://localhost:8000").unwrap(),
            credit_program: quotes::test_credit_program_binding(),
            authorization_verifier: crate::authorization::test_authorization_verifier(),
            credit_evidence: None,
        });

        assert!(reissue_enquire_quote(
            State(service),
            Json(wire_quotes::SignedReissueEnquireRequestV1 { content, signature }),
        )
        .await
        .is_err());
    }
}
