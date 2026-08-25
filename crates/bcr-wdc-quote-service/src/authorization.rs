use base64::{engine::general_purpose, Engine as _};
use bcr_common::wire::quotes as wire_quotes;
use bitcoin::{
    hashes::{sha256, Hash as _},
    Amount,
};
use chrono::{DateTime, NaiveDate, Utc};
use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};
use unicode_normalization::UnicodeNormalization;

use crate::{
    error::{Error, Result},
    quotes::Quote,
    TStamp,
};

const MAX_SATOSHIS: u64 = 2_100_000_000_000_000;
pub const AUTHORIZATION_ACTION: &str = "request_to_mint";

#[derive(Clone, Debug)]
pub struct AuthorizationVerifier {
    mint_id: String,
    key_id: String,
    public_key: VerifyingKey,
}

#[derive(Debug)]
pub struct VerifiedOffer {
    pub authorization_digest: String,
    pub operation_id: String,
    pub discounted: Amount,
    pub expiration: TStamp,
    pub authorization: wire_quotes::CreditAuthorizationEnvelope,
}

impl AuthorizationVerifier {
    pub fn new(mint_id: String, key_id: String, public_key_base64url: String) -> Result<Self> {
        validate_text(&mint_id)?;
        validate_text(&key_id)?;
        let bytes = general_purpose::URL_SAFE_NO_PAD
            .decode(&public_key_base64url)
            .or_else(|_| general_purpose::URL_SAFE.decode(&public_key_base64url))
            .map_err(|_| invalid())?;
        let public_key =
            VerifyingKey::from_bytes(bytes.as_slice().try_into().map_err(|_| invalid())?)
                .map_err(|_| invalid())?;
        Ok(Self {
            mint_id,
            key_id,
            public_key,
        })
    }

    pub fn verify(
        &self,
        signed: wire_quotes::SignedCreditAuthorizationEnvelope,
        quote: &Quote,
        now: TStamp,
    ) -> Result<VerifiedOffer> {
        let authorization = signed.authorization;
        validate_envelope(&authorization)?;
        if signed.signature_algorithm != "Ed25519"
            || authorization.key_id != self.key_id
            || authorization.mint_id != self.mint_id
        {
            return Err(invalid());
        }

        let canonical = canonical_authorization(&authorization);
        let authorization_digest = digest(&canonical);
        if signed.authorization_digest != authorization_digest {
            return Err(invalid());
        }
        let signature_bytes = general_purpose::STANDARD
            .decode(&signed.signature)
            .map_err(|_| invalid())?;
        let signature = Signature::from_slice(&signature_bytes).map_err(|_| invalid())?;
        self.public_key
            .verify(&canonical, &signature)
            .map_err(|_| invalid())?;

        let issued_at = parse_datetime(&authorization.issued_at)?;
        let expires_at = parse_datetime(&authorization.expires_at)?;
        if issued_at > now || expires_at <= now || expires_at <= issued_at {
            return Err(invalid());
        }

        let credit_program = quote
            .credit_program()
            .ok_or(Error::CreditProgramNotBound(quote.id))?;
        let holder_ref = quote.bill.current_holder.node_id().to_string();
        let acceptor_ref = quote.bill.drawee.node_id.to_string();
        let bill_id = quote.bill.id.to_string();
        let maturity_date = quote.bill.maturity_date.to_string();
        let face_value_sat = quote.bill.sum.to_sat().to_string();
        let bill_state_digest = digest(&canonical_bill_state(
            &bill_id,
            &holder_ref,
            &acceptor_ref,
            &face_value_sat,
            &maturity_date,
        ));

        if authorization.schema_version != "credit-authorization-v7"
            || !authorization.synthetic
            || authorization.action != AUTHORIZATION_ACTION
            || authorization.mint_quote_id != quote.id.to_string()
            || authorization.credit_program_version != credit_program.version()
            || authorization.credit_program_digest != credit_program.digest()
            || authorization.bill_id != bill_id
            || authorization.bill_state_digest != bill_state_digest
            || authorization.holder_ref != holder_ref
            || authorization.acceptor_ref != acceptor_ref
            || authorization.terms.bill_sum_sat != face_value_sat
            || authorization.terms.endorsement_exposure_sat != face_value_sat
            || authorization.terms.maturity_date != maturity_date
        {
            return Err(invalid());
        }

        let expiration_date = parse_date(&authorization.terms.offer_expires_on)?;
        let expiration = expiration_date
            .and_hms_milli_opt(23, 59, 59, 999)
            .ok_or_else(invalid)?
            .and_utc();
        if expiration
            > quote
                .bill
                .maturity_date
                .and_hms_opt(23, 59, 59)
                .ok_or_else(invalid)?
                .and_utc()
            || expires_at > expiration
        {
            return Err(invalid());
        }

        Ok(VerifiedOffer {
            authorization_digest,
            operation_id: digest(&canonical_operation(&authorization)),
            discounted: Amount::from_sat(parse_sat(&authorization.terms.discounted_sat)?),
            expiration,
            authorization,
        })
    }
}

#[cfg(test)]
pub(crate) fn test_authorization_verifier() -> AuthorizationVerifier {
    AuthorizationVerifier::new(
        String::from("local-wildcat"),
        String::from("synthetic-ed25519-v1"),
        String::from("jn__htbnO4jauBDTN5Oeby-2uOylC6skT2-jm8mXiNc"),
    )
    .expect("valid synthetic authorization verifier")
}

fn invalid() -> Error {
    Error::CreditAuthorizationInvalid
}

fn validate_text(value: &str) -> Result<()> {
    if value.is_empty() || value.chars().count() > 200 || value.nfc().collect::<String>() != value {
        return Err(invalid());
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<()> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(invalid());
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid());
    }
    Ok(())
}

fn parse_sat(value: &str) -> Result<u64> {
    if value != "0" && value.starts_with('0') {
        return Err(invalid());
    }
    let amount = value.parse::<u64>().map_err(|_| invalid())?;
    if amount > MAX_SATOSHIS {
        return Err(invalid());
    }
    Ok(amount)
}

fn parse_date(value: &str) -> Result<NaiveDate> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| invalid())?;
    if date.to_string() != value {
        return Err(invalid());
    }
    Ok(date)
}

fn parse_datetime(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| invalid())
}

fn validate_envelope(value: &wire_quotes::CreditAuthorizationEnvelope) -> Result<()> {
    for text in [
        &value.key_id,
        &value.mint_id,
        &value.mint_quote_id,
        &value.credit_program_version,
        &value.case_id,
        &value.bill_id,
        &value.holder_ref,
        &value.acceptor_ref,
        &value.policy_pack_version,
        &value.calculation_version,
        &value.operator_id,
        &value.action,
    ] {
        validate_text(text)?;
    }
    for digest_value in [
        &value.credit_program_digest,
        &value.bill_state_digest,
        &value.decision_snapshot_digest,
        &value.decision_result_digest,
        &value.policy_pack_digest,
    ] {
        validate_digest(digest_value)?;
    }
    uuid::Uuid::parse_str(&value.nonce).map_err(|_| invalid())?;
    parse_date(&value.terms.maturity_date)?;
    parse_date(&value.terms.offer_expires_on)?;
    if value.terms.tenor_days == 0
        || value.terms.annual_discount_bps > 20_000
        || value.terms.effective_annual_bps > 20_000
        || value.terms.fee_ratio_bps > 10_000
    {
        return Err(invalid());
    }

    let sum = u128::from(parse_sat(&value.terms.bill_sum_sat)?);
    let discounted = u128::from(parse_sat(&value.terms.discounted_sat)?);
    let applied = u128::from(parse_sat(&value.terms.applied_discount_sat)?);
    let operating = u128::from(parse_sat(&value.terms.operating_cost_sat)?);
    let effective_fee = u128::from(parse_sat(&value.terms.effective_fee_sat)?);
    let exposure = u128::from(parse_sat(&value.terms.endorsement_exposure_sat)?);
    let fee = applied + operating;
    if sum == 0
        || discounted == 0
        || exposure != sum
        || sum != discounted + fee
        || effective_fee != fee
        || u128::from(value.terms.fee_ratio_bps) != (fee * 10_000).div_ceil(sum)
        || u128::from(value.terms.effective_annual_bps)
            != (fee * 10_000 * 360).div_ceil(discounted * u128::from(value.terms.tenor_days))
        || value.terms.offer_expires_on > value.terms.maturity_date
    {
        return Err(invalid());
    }
    Ok(())
}

fn canonical_authorization(value: &wire_quotes::CreditAuthorizationEnvelope) -> Vec<u8> {
    encode_fields(&[
        "AI-CREDIT-AUTHORIZATION-V7",
        &value.schema_version,
        &value.key_id,
        &value.mint_id,
        &value.mint_quote_id,
        &value.credit_program_version,
        &value.credit_program_digest,
        &value.case_id,
        &value.bill_id,
        &value.bill_state_digest,
        &value.holder_ref,
        &value.acceptor_ref,
        &value.decision_snapshot_digest,
        &value.decision_result_digest,
        &value.policy_pack_digest,
        &value.policy_pack_version,
        &value.calculation_version,
        &value.terms.bill_sum_sat,
        &value.terms.discounted_sat,
        &value.terms.applied_discount_sat,
        &value.terms.operating_cost_sat,
        &value.terms.effective_fee_sat,
        &value.terms.endorsement_exposure_sat,
        &value.terms.maturity_date,
        &value.terms.offer_expires_on,
        &value.terms.tenor_days.to_string(),
        &value.terms.annual_discount_bps.to_string(),
        &value.terms.effective_annual_bps.to_string(),
        &value.terms.fee_ratio_bps.to_string(),
        &value.operator_id,
        &value.issued_at,
        &value.expires_at,
        &value.nonce,
        &value.action,
        "true",
    ])
}

fn canonical_operation(value: &wire_quotes::CreditAuthorizationEnvelope) -> Vec<u8> {
    encode_fields(&[
        "AI-CREDIT-OPERATION-V1",
        &value.mint_id,
        &value.action,
        &value.nonce,
    ])
}

fn canonical_bill_state(
    bill_id: &str,
    holder_ref: &str,
    acceptor_ref: &str,
    face_value_sat: &str,
    maturity_date: &str,
) -> Vec<u8> {
    encode_fields(&[
        "AI-CREDIT-MINT-BILL-STATE-V1",
        "mint-observed-bill-state-v1",
        bill_id,
        holder_ref,
        acceptor_ref,
        face_value_sat,
        maturity_date,
    ])
}

pub fn offer_result_digest(quote_id: uuid::Uuid, discounted: Amount, expiration: TStamp) -> String {
    digest(&encode_fields(&[
        "AI-CREDIT-WILDCAT-OFFER-RESULT-V1",
        &quote_id.to_string(),
        &discounted.to_sat().to_string(),
        &expiration.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    ]))
}

fn encode_fields(fields: &[&str]) -> Vec<u8> {
    let mut output = Vec::new();
    for field in fields {
        let bytes = field.as_bytes();
        output.extend_from_slice(
            &u32::try_from(bytes.len())
                .expect("authorization field length is bounded")
                .to_be_bytes(),
        );
        output.extend_from_slice(bytes);
    }
    output
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256::Hash::hash(bytes))
}

#[cfg(test)]
mod tests {
    use base64::{engine::general_purpose, Engine as _};
    use bcr_common::wire::quotes as wire_quotes;
    use bitcoin::{hashes::Hash as _, Amount};
    use ed25519_dalek::{Signer as _, SigningKey};

    use super::*;

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(
            &sha256::Hash::hash(b"ai-credit synthetic ed25519 conformance seed v1").to_byte_array(),
        )
    }

    fn signed_for(quote: &Quote) -> wire_quotes::SignedCreditAuthorizationEnvelope {
        let holder_ref = quote.bill.current_holder.node_id().to_string();
        let acceptor_ref = quote.bill.drawee.node_id.to_string();
        let bill_id = quote.bill.id.to_string();
        let maturity_date = quote.bill.maturity_date.to_string();
        let authorization = wire_quotes::CreditAuthorizationEnvelope {
            schema_version: String::from("credit-authorization-v7"),
            key_id: String::from("synthetic-ed25519-v1"),
            mint_id: String::from("local-wildcat"),
            mint_quote_id: quote.id.to_string(),
            credit_program_version: quote.credit_program().unwrap().version().to_owned(),
            credit_program_digest: quote.credit_program().unwrap().digest().to_owned(),
            case_id: String::from("synthetic-case-a"),
            bill_id: bill_id.clone(),
            bill_state_digest: digest(&canonical_bill_state(
                &bill_id,
                &holder_ref,
                &acceptor_ref,
                "8000000",
                &maturity_date,
            )),
            holder_ref,
            acceptor_ref,
            decision_snapshot_digest: format!("sha256:{}", "a".repeat(64)),
            decision_result_digest: format!("sha256:{}", "b".repeat(64)),
            policy_pack_digest: format!("sha256:{}", "c".repeat(64)),
            policy_pack_version: String::from("synthetic-policy-v1"),
            calculation_version: String::from("deterministic-credit-core-v9"),
            terms: wire_quotes::CreditAuthorizationTerms {
                bill_sum_sat: String::from("8000000"),
                discounted_sat: String::from("7734000"),
                applied_discount_sat: String::from("216000"),
                operating_cost_sat: String::from("50000"),
                effective_fee_sat: String::from("266000"),
                endorsement_exposure_sat: String::from("8000000"),
                maturity_date,
                offer_expires_on: String::from("2026-08-12"),
                tenor_days: 180,
                annual_discount_bps: 540,
                effective_annual_bps: 688,
                fee_ratio_bps: 333,
            },
            operator_id: String::from("synthetic-operator-a"),
            issued_at: String::from("2026-08-10T12:00:00.000Z"),
            expires_at: String::from("2026-08-10T12:15:00.000Z"),
            nonce: String::from("00000000-0000-4000-8000-000000000001"),
            action: String::from(AUTHORIZATION_ACTION),
            synthetic: true,
        };
        let bytes = canonical_authorization(&authorization);
        wire_quotes::SignedCreditAuthorizationEnvelope {
            authorization,
            authorization_digest: digest(&bytes),
            signature_algorithm: String::from("Ed25519"),
            signature: general_purpose::STANDARD.encode(signing_key().sign(&bytes).to_bytes()),
        }
    }

    #[test]
    fn verifies_frozen_vector_bytes_and_rejects_tampering() {
        let vector: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/authorization/authorization-v7.json"
        ))
        .unwrap();
        let authorization: wire_quotes::CreditAuthorizationEnvelope =
            serde_json::from_value(vector["authorization"].clone()).unwrap();
        let bytes = canonical_authorization(&authorization);
        assert_eq!(
            general_purpose::STANDARD.encode(&bytes),
            vector["expected"]["canonicalBase64"].as_str().unwrap()
        );
        assert_eq!(
            digest(&bytes),
            vector["expected"]["authorizationDigest"].as_str().unwrap()
        );
        let signature = general_purpose::STANDARD
            .decode(vector["expected"]["signature"].as_str().unwrap())
            .unwrap();
        signing_key()
            .verifying_key()
            .verify(&bytes, &Signature::from_slice(&signature).unwrap())
            .unwrap();
    }

    #[test]
    fn binds_offer_to_the_actual_quote_and_bill_state() {
        let mut bill = crate::quotes::BillInfo::random();
        bill.sum = Amount::from_sat(8_000_000);
        bill.maturity_date = NaiveDate::from_ymd_opt(2027, 2, 6).unwrap();
        let quote = Quote::new(
            bill,
            bcr_wdc_utils::keys::test_utils::publics()[0],
            TStamp::default(),
            crate::quotes::test_credit_program_binding(),
        );
        let signed = signed_for(&quote);
        let now = DateTime::parse_from_rfc3339("2026-08-10T12:05:00.000Z")
            .unwrap()
            .with_timezone(&Utc);
        let verified = test_authorization_verifier()
            .verify(signed.clone(), &quote, now)
            .unwrap();
        assert_eq!(verified.discounted, Amount::from_sat(7_734_000));

        let mut tampered = signed;
        tampered.authorization.terms.discounted_sat = String::from("7733999");
        assert!(matches!(
            test_authorization_verifier().verify(tampered, &quote, now),
            Err(Error::CreditAuthorizationInvalid)
        ));
    }
}
