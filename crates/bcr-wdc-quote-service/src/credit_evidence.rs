use anyhow::anyhow;
use base64::{engine::general_purpose, Engine as _};
use bcr_common::wire::quotes as wire_quotes;
use bcr_wdc_utils::surreal;
use bitcoin::hashes::{sha256, Hash};
use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};
use surrealdb::{engine::any::Any, Surreal};
use unicode_normalization::UnicodeNormalization;

use crate::{
    error::{Error, Result},
    quotes::Quote,
};

#[derive(Clone, Debug)]
pub struct Settings {
    pub mint_id: String,
    pub risk_methodology_version: String,
    pub risk_assessed_by: String,
    pub risk_authority_key_id: String,
    pub risk_authority_public_key: String,
    pub allow_synthetic: bool,
}

impl Settings {
    pub fn validate(self) -> Result<Self> {
        bounded(&self.mint_id, "Mint id", 1, 200)?;
        bounded(
            &self.risk_methodology_version,
            "risk methodology version",
            1,
            200,
        )?;
        bounded(&self.risk_assessed_by, "risk assessor", 1, 200)?;
        bounded(&self.risk_authority_key_id, "risk authority key id", 1, 200)?;
        parse_public_key(&self.risk_authority_public_key)?;
        Ok(self)
    }
}

#[derive(Clone)]
pub struct Store {
    db: Surreal<Any>,
    settings: Settings,
}

fn bounded(value: &str, field: &str, min: usize, max: usize) -> Result<()> {
    if value.trim() != value
        || value.len() < min
        || value.len() > max
        || value.chars().any(char::is_control)
        || value.nfc().collect::<String>() != value
    {
        return Err(Error::InvalidInput(format!(
            "{field} must contain {min} to {max} non-control, non-whitespace-padded characters"
        )));
    }
    Ok(())
}

fn basis_digest(value: &str) -> String {
    format!("sha256:{}", sha256::Hash::hash(value.as_bytes()))
}

fn parse_public_key(value: &str) -> Result<VerifyingKey> {
    let bytes = general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .or_else(|_| general_purpose::URL_SAFE.decode(value))
        .map_err(|_| Error::InvalidInput(String::from("invalid evidence authority public key")))?;
    VerifyingKey::from_bytes(
        bytes.as_slice().try_into().map_err(|_| {
            Error::InvalidInput(String::from("invalid evidence authority public key"))
        })?,
    )
    .map_err(|_| Error::InvalidInput(String::from("invalid evidence authority public key")))
}

fn encode_fields(fields: &[&str]) -> Vec<u8> {
    let mut output = Vec::new();
    for field in fields {
        let bytes = field.as_bytes();
        output.extend_from_slice(
            &u32::try_from(bytes.len())
                .expect("evidence field length is bounded")
                .to_be_bytes(),
        );
        output.extend_from_slice(bytes);
    }
    output
}

fn risk_bytes(value: &wire_quotes::AcceptorRiskAuthorityEvidence) -> Vec<u8> {
    let mut fields = vec![
        String::from("BITCREDIT-MINT-ACCEPTOR-RISK-EVIDENCE-V1"),
        value.schema_version.clone(),
        value.key_id.clone(),
        value.acceptor_ref.clone(),
        value.probability_of_default_bps.to_string(),
        value.loss_given_default_bps.to_string(),
        value.evidence_state.clone(),
        value.methodology_version.clone(),
        value.assessed_by.clone(),
        value.assessed_at.to_string(),
        value.valid_through.to_string(),
        value.synthetic.to_string(),
        value.evidence_refs.len().to_string(),
    ];
    fields.extend(value.evidence_refs.iter().cloned());
    encode_fields(&fields.iter().map(String::as_str).collect::<Vec<_>>())
}

fn verify_signature(
    bytes: &[u8],
    digest: &str,
    algorithm: &str,
    signature: &str,
    key: &VerifyingKey,
) -> Result<()> {
    if algorithm != "Ed25519" || digest != basis_digest_bytes(bytes) {
        return Err(Error::InvalidInput(String::from(
            "evidence authority signature is invalid",
        )));
    }
    let signature = general_purpose::STANDARD
        .decode(signature)
        .ok()
        .and_then(|bytes| Signature::from_slice(&bytes).ok())
        .ok_or_else(|| {
            Error::InvalidInput(String::from("evidence authority signature is invalid"))
        })?;
    key.verify(bytes, &signature)
        .map_err(|_| Error::InvalidInput(String::from("evidence authority signature is invalid")))
}

fn basis_digest_bytes(value: &[u8]) -> String {
    format!("sha256:{}", sha256::Hash::hash(value))
}

fn validate_evidence_refs(values: &[String]) -> Result<()> {
    if values.is_empty() || values.len() > 20 {
        return Err(Error::InvalidInput(String::from(
            "authority evidence must contain 1 to 20 source references",
        )));
    }
    for value in values {
        bounded(value, "authority evidence source reference", 1, 500)?;
    }
    Ok(())
}

impl Store {
    const ACCEPTOR_TABLE: &'static str = "credit_acceptor_risk_evidence";

    pub async fn new(cfg: surreal::DBConnConfig, settings: Settings) -> surrealdb::Result<Self> {
        let db = Surreal::<Any>::init();
        db.connect(cfg.connection).await?;
        db.use_ns(cfg.namespace).await?;
        db.use_db(cfg.database).await?;
        Ok(Self { db, settings })
    }

    async fn latest_acceptor(
        &self,
        acceptor_ref: &str,
    ) -> Result<Option<wire_quotes::AcceptorRiskEvidence>> {
        self.db
            .query("SELECT schemaVersion, evidenceId, signedEvidence, operatorId, writtenBasisDigest, recordedAt, verifiedAt FROM type::table($table) WHERE signedEvidence.evidence.acceptorRef == $acceptor_ref ORDER BY recordedAt DESC LIMIT 1")
            .bind(("table", Self::ACCEPTOR_TABLE))
            .bind(("acceptor_ref", acceptor_ref.to_owned()))
            .await
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?
            .take::<Vec<wire_quotes::AcceptorRiskEvidence>>(0)
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))
            .map(|records| records.into_iter().next())
    }

    fn verify_risk(
        &self,
        signed: &wire_quotes::SignedAcceptorRiskEvidence,
        acceptor_ref: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        let evidence = &signed.evidence;
        if evidence.schema_version != "mint-acceptor-risk-authority-evidence-v1"
            || evidence.key_id != self.settings.risk_authority_key_id
            || evidence.acceptor_ref != acceptor_ref
            || evidence.evidence_state != "corroborated"
            || evidence.methodology_version != self.settings.risk_methodology_version
            || evidence.assessed_by != self.settings.risk_assessed_by
            || (evidence.synthetic && !self.settings.allow_synthetic)
            || evidence.probability_of_default_bps > 10_000
            || evidence.loss_given_default_bps > 10_000
        {
            return Err(Error::InvalidInput(String::from(
                "acceptor risk authority evidence does not match the Mint trust policy",
            )));
        }
        validate_evidence_refs(&evidence.evidence_refs)?;
        if evidence.assessed_at > now.date_naive()
            || evidence.valid_through < now.date_naive()
            || evidence.valid_through < evidence.assessed_at
        {
            return Err(Error::InvalidInput(String::from(
                "acceptor risk authority evidence is not currently valid",
            )));
        }
        verify_signature(
            &risk_bytes(evidence),
            &signed.evidence_digest,
            &signed.signature_algorithm,
            &signed.signature,
            &parse_public_key(&self.settings.risk_authority_public_key)?,
        )
    }

    pub async fn for_quote(&self, quote: &Quote) -> Result<wire_quotes::MintCreditEvidence> {
        let acceptor_ref = quote.bill.drawee.node_id.to_string();
        let acceptor_risk = self.latest_acceptor(&acceptor_ref).await?;
        if let Some(record) = &acceptor_risk {
            self.verify_risk(&record.signed_evidence, &acceptor_ref, chrono::Utc::now())?;
        }
        Ok(wire_quotes::MintCreditEvidence {
            schema_version: String::from("mint-credit-evidence-v2"),
            mint_id: self.settings.mint_id.clone(),
            acceptor_ref,
            acceptor_risk,
        })
    }

    pub async fn record_acceptor(
        &self,
        quote: &Quote,
        command: wire_quotes::AcceptorRiskEvidenceCommand,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<wire_quotes::AcceptorRiskEvidence> {
        let request = command.request;
        bounded(&command.operator_id, "operator id", 1, 200)?;
        bounded(&request.written_basis, "written basis", 20, 2_000)?;
        let signed = request.signed_evidence;
        let acceptor_ref = quote.bill.drawee.node_id.to_string();
        self.verify_risk(&signed, &acceptor_ref, now)?;
        let written_basis_digest = basis_digest(&request.written_basis);
        if let Some(existing) = self.latest_acceptor(&acceptor_ref).await? {
            if existing.operator_id == command.operator_id
                && existing.signed_evidence.evidence_digest == signed.evidence_digest
                && existing.written_basis_digest == written_basis_digest
            {
                return Ok(existing);
            }
        }
        let evidence_id = uuid::Uuid::new_v4();
        let record = wire_quotes::AcceptorRiskEvidence {
            schema_version: String::from("mint-acceptor-risk-record-v2"),
            evidence_id,
            signed_evidence: signed,
            operator_id: command.operator_id,
            written_basis_digest,
            recorded_at: now,
            verified_at: now,
        };
        let rid = surrealdb::RecordId::from_table_key(Self::ACCEPTOR_TABLE, evidence_id);
        self.db
            .query("CREATE $rid CONTENT $record")
            .bind(("rid", rid))
            .bind(("record", record.clone()))
            .await
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?
            .check()
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose;
    use bcr_common::wire::quotes::{
        AcceptorRiskAuthorityEvidence, AcceptorRiskEvidenceCommand, AcceptorRiskEvidenceRequest,
        SignedAcceptorRiskEvidence,
    };
    use bcr_wdc_utils::keys::test_utils as keys_test;
    use ed25519_dalek::{Signer as _, SigningKey};

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(
            &sha256::Hash::hash(b"local testnet credit evidence authority v1").to_byte_array(),
        )
    }

    async fn store() -> Store {
        Store::new(
            surreal::DBConnConfig {
                connection: String::from("mem://"),
                namespace: String::from("test"),
                database: String::from("test"),
            },
            Settings {
                mint_id: String::from("local-wildcat"),
                risk_methodology_version: String::from("risk-v1"),
                risk_assessed_by: String::from("risk-owner"),
                risk_authority_key_id: String::from("testnet-risk-authority-v1"),
                risk_authority_public_key: general_purpose::URL_SAFE_NO_PAD
                    .encode(signing_key().verifying_key().as_bytes()),
                allow_synthetic: true,
            },
        )
        .await
        .unwrap()
    }

    fn signed_risk(
        quote: &Quote,
        now: chrono::DateTime<chrono::Utc>,
    ) -> SignedAcceptorRiskEvidence {
        let evidence = AcceptorRiskAuthorityEvidence {
            schema_version: String::from("mint-acceptor-risk-authority-evidence-v1"),
            key_id: String::from("testnet-risk-authority-v1"),
            acceptor_ref: quote.bill.drawee.node_id.to_string(),
            probability_of_default_bps: 600,
            loss_given_default_bps: 4_000,
            evidence_state: String::from("corroborated"),
            methodology_version: String::from("risk-v1"),
            assessed_by: String::from("risk-owner"),
            assessed_at: now.date_naive(),
            valid_through: now.date_naive() + chrono::Days::new(30),
            evidence_refs: vec![String::from("risk-book-2026-08")],
            synthetic: true,
        };
        let bytes = risk_bytes(&evidence);
        SignedAcceptorRiskEvidence {
            evidence,
            evidence_digest: basis_digest_bytes(&bytes),
            signature_algorithm: String::from("Ed25519"),
            signature: general_purpose::STANDARD.encode(signing_key().sign(&bytes).to_bytes()),
        }
    }

    fn quote() -> Quote {
        Quote::new(
            crate::quotes::BillInfo::random(),
            keys_test::publics()[0],
            chrono::Utc::now(),
            crate::quotes::test_credit_program_binding(),
        )
    }

    #[tokio::test]
    async fn records_quote_bound_evidence_append_only_and_idempotently() {
        let store = store().await;
        let quote = quote();
        let now = chrono::Utc::now();
        let acceptor = AcceptorRiskEvidenceCommand {
            operator_id: String::from("operator-a"),
            request: AcceptorRiskEvidenceRequest {
                signed_evidence: signed_risk(&quote, now),
                written_basis: String::from("Reviewed against the current Mint risk book."),
            },
        };
        let first = store
            .record_acceptor(&quote, acceptor.clone(), now)
            .await
            .unwrap();
        let replay = store
            .record_acceptor(&quote, acceptor, now + chrono::Duration::seconds(1))
            .await
            .unwrap();
        assert_eq!(first.evidence_id, replay.evidence_id);

        let snapshot = store.for_quote(&quote).await.unwrap();
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["schemaVersion"], "mint-credit-evidence-v2");
        assert_eq!(
            json["acceptorRisk"]["schemaVersion"],
            "mint-acceptor-risk-record-v2"
        );
        assert!(json.get("acceptor_risk").is_none());
        assert_eq!(
            snapshot
                .acceptor_risk
                .unwrap()
                .signed_evidence
                .evidence
                .probability_of_default_bps,
            600
        );
    }

    #[tokio::test]
    async fn rejects_tampered_or_untrusted_authority_evidence() {
        let store = store().await;
        let quote = quote();
        let now = chrono::Utc::now();
        let mut signed = signed_risk(&quote, now);
        signed.evidence.probability_of_default_bps += 1;
        let error = store
            .record_acceptor(
                &quote,
                AcceptorRiskEvidenceCommand {
                    operator_id: String::from("operator-a"),
                    request: AcceptorRiskEvidenceRequest {
                        signed_evidence: signed,
                        written_basis: String::from("Reviewed against the current Mint risk book."),
                    },
                },
                now,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, Error::InvalidInput(_)));
    }
}
