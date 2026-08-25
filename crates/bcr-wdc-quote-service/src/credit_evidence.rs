use anyhow::anyhow;
use bcr_common::wire::quotes as wire_quotes;
use bcr_wdc_utils::surreal;
use bitcoin::hashes::{sha256, Hash};
use surrealdb::{engine::any::Any, Surreal};

use crate::{
    error::{Error, Result},
    quotes::Quote,
};

const MAX_SATOSHIS: u64 = 2_100_000_000_000_000;

#[derive(Clone, Debug)]
pub struct Settings {
    pub mint_id: String,
    pub risk_methodology_version: String,
    pub risk_assessed_by: String,
    pub capacity_methodology_version: String,
    pub capacity_assessed_by: String,
    pub synthetic: bool,
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
        bounded(
            &self.capacity_methodology_version,
            "capacity methodology version",
            1,
            200,
        )?;
        bounded(&self.capacity_assessed_by, "capacity assessor", 1, 200)?;
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
    {
        return Err(Error::InvalidInput(format!(
            "{field} must contain {min} to {max} non-control, non-whitespace-padded characters"
        )));
    }
    Ok(())
}

fn satoshis(value: &str, field: &str) -> Result<u64> {
    if value != "0" && value.starts_with('0') {
        return Err(Error::InvalidInput(format!(
            "{field} must be a canonical non-negative satoshi string"
        )));
    }
    let amount = value.parse::<u64>().map_err(|_| {
        Error::InvalidInput(format!(
            "{field} must be a canonical non-negative satoshi string"
        ))
    })?;
    if amount > MAX_SATOSHIS {
        return Err(Error::InvalidInput(format!(
            "{field} exceeds the maximum satoshi supply"
        )));
    }
    Ok(amount)
}

fn basis_digest(value: &str) -> String {
    format!("sha256:{}", sha256::Hash::hash(value.as_bytes()))
}

impl Store {
    const ACCEPTOR_TABLE: &'static str = "credit_acceptor_risk_evidence";
    const CAPACITY_TABLE: &'static str = "credit_mint_capacity_evidence";

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
            .query("SELECT schemaVersion, evidenceId, acceptorRef, probabilityOfDefaultBps, lossGivenDefaultBps, evidenceState, methodologyVersion, assessedBy, assessedAt, validThrough, evidenceRefs, operatorId, recordedAt, synthetic FROM type::table($table) WHERE acceptorRef == $acceptor_ref ORDER BY recordedAt DESC LIMIT 1")
            .bind(("table", Self::ACCEPTOR_TABLE))
            .bind(("acceptor_ref", acceptor_ref.to_owned()))
            .await
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?
            .take::<Vec<wire_quotes::AcceptorRiskEvidence>>(0)
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))
            .map(|records| records.into_iter().next())
    }

    async fn latest_capacity(&self) -> Result<Option<wire_quotes::MintCapacityEvidence>> {
        self.db
            .query("SELECT schemaVersion, evidenceId, mintId, existingExposureSat, exposureLimitSat, evidenceState, methodologyVersion, assessedBy, assessedAt, validThrough, evidenceRefs, operatorId, recordedAt, synthetic FROM type::table($table) WHERE mintId == $mint_id ORDER BY recordedAt DESC LIMIT 1")
            .bind(("table", Self::CAPACITY_TABLE))
            .bind(("mint_id", self.settings.mint_id.clone()))
            .await
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?
            .take::<Vec<wire_quotes::MintCapacityEvidence>>(0)
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))
            .map(|records| records.into_iter().next())
    }

    pub async fn for_quote(&self, quote: &Quote) -> Result<wire_quotes::MintCreditEvidence> {
        let acceptor_ref = quote.bill.drawee.node_id.to_string();
        let (acceptor_risk, mint_capacity) =
            tokio::try_join!(self.latest_acceptor(&acceptor_ref), self.latest_capacity())?;
        Ok(wire_quotes::MintCreditEvidence {
            schema_version: String::from("mint-credit-evidence-v1"),
            mint_id: self.settings.mint_id.clone(),
            acceptor_ref,
            acceptor_risk,
            mint_capacity,
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
        bounded(&request.source_reference, "source reference", 1, 200)?;
        bounded(&request.written_basis, "written basis", 20, 2_000)?;
        if request.probability_of_default_bps > 10_000 || request.loss_given_default_bps > 10_000 {
            return Err(Error::InvalidInput(String::from(
                "PD and LGD must be between 0 and 10000 basis points",
            )));
        }
        let assessed_at = now.date_naive();
        if request.valid_through < assessed_at {
            return Err(Error::InvalidInput(String::from(
                "acceptor risk evidence is already expired",
            )));
        }
        let acceptor_ref = quote.bill.drawee.node_id.to_string();
        let basis_ref = format!("basis:{}", basis_digest(&request.written_basis));
        if let Some(existing) = self.latest_acceptor(&acceptor_ref).await? {
            if existing.operator_id == command.operator_id
                && existing.probability_of_default_bps == request.probability_of_default_bps
                && existing.loss_given_default_bps == request.loss_given_default_bps
                && existing.valid_through == request.valid_through
                && existing.evidence_refs.first() == Some(&request.source_reference)
                && existing.evidence_refs.contains(&basis_ref)
            {
                return Ok(existing);
            }
        }
        let evidence_id = uuid::Uuid::new_v4();
        let record = wire_quotes::AcceptorRiskEvidence {
            schema_version: String::from("mint-acceptor-risk-evidence-v1"),
            evidence_id,
            acceptor_ref,
            probability_of_default_bps: request.probability_of_default_bps,
            loss_given_default_bps: request.loss_given_default_bps,
            evidence_state: String::from("corroborated"),
            methodology_version: self.settings.risk_methodology_version.clone(),
            assessed_by: self.settings.risk_assessed_by.clone(),
            assessed_at,
            valid_through: request.valid_through,
            evidence_refs: vec![
                request.source_reference,
                format!("mint-risk-record:{evidence_id}"),
                format!("operator:{}", command.operator_id),
                basis_ref,
            ],
            operator_id: command.operator_id,
            recorded_at: now,
            synthetic: self.settings.synthetic,
        };
        let rid = surrealdb::RecordId::from_table_key(Self::ACCEPTOR_TABLE, evidence_id);
        self.db
            .query("CREATE $rid CONTENT $record")
            .bind(("rid", rid))
            .bind(("record", record.clone()))
            .await
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?;
        Ok(record)
    }

    pub async fn record_capacity(
        &self,
        command: wire_quotes::MintCapacityEvidenceCommand,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<wire_quotes::MintCapacityEvidence> {
        let request = command.request;
        bounded(&command.operator_id, "operator id", 1, 200)?;
        bounded(&request.source_reference, "source reference", 1, 200)?;
        bounded(&request.written_basis, "written basis", 20, 2_000)?;
        let existing_exposure = satoshis(&request.existing_exposure_sat, "existing exposure")?;
        let exposure_limit = satoshis(&request.exposure_limit_sat, "exposure limit")?;
        if exposure_limit == 0 {
            return Err(Error::InvalidInput(String::from(
                "exposure limit must be positive",
            )));
        }
        let assessed_at = now.date_naive();
        if request.valid_through < assessed_at {
            return Err(Error::InvalidInput(String::from(
                "Mint capacity evidence is already expired",
            )));
        }
        let basis_ref = format!("basis:{}", basis_digest(&request.written_basis));
        if let Some(existing) = self.latest_capacity().await? {
            if existing.operator_id == command.operator_id
                && existing.existing_exposure_sat == request.existing_exposure_sat
                && existing.exposure_limit_sat == request.exposure_limit_sat
                && existing.valid_through == request.valid_through
                && existing.evidence_refs.first() == Some(&request.source_reference)
                && existing.evidence_refs.contains(&basis_ref)
            {
                return Ok(existing);
            }
        }
        let evidence_id = uuid::Uuid::new_v4();
        let record = wire_quotes::MintCapacityEvidence {
            schema_version: String::from("mint-capacity-evidence-v1"),
            evidence_id,
            mint_id: self.settings.mint_id.clone(),
            existing_exposure_sat: existing_exposure.to_string(),
            exposure_limit_sat: exposure_limit.to_string(),
            evidence_state: String::from("corroborated"),
            methodology_version: self.settings.capacity_methodology_version.clone(),
            assessed_by: self.settings.capacity_assessed_by.clone(),
            assessed_at,
            valid_through: request.valid_through,
            evidence_refs: vec![
                request.source_reference,
                format!("mint-capacity-record:{evidence_id}"),
                format!("operator:{}", command.operator_id),
                basis_ref,
            ],
            operator_id: command.operator_id,
            recorded_at: now,
            synthetic: self.settings.synthetic,
        };
        let rid = surrealdb::RecordId::from_table_key(Self::CAPACITY_TABLE, evidence_id);
        self.db
            .query("CREATE $rid CONTENT $record")
            .bind(("rid", rid))
            .bind(("record", record.clone()))
            .await
            .map_err(|error| Error::QuotesRepository(anyhow!(error)))?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_common::wire::quotes::{
        AcceptorRiskEvidenceCommand, AcceptorRiskEvidenceRequest, MintCapacityEvidenceCommand,
        MintCapacityEvidenceRequest,
    };
    use bcr_wdc_utils::keys::test_utils as keys_test;

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
                capacity_methodology_version: String::from("capacity-v1"),
                capacity_assessed_by: String::from("mint-owner"),
                synthetic: true,
            },
        )
        .await
        .unwrap()
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
                probability_of_default_bps: 600,
                loss_given_default_bps: 4_000,
                source_reference: String::from("risk-book-2026-08"),
                valid_through: now.date_naive() + chrono::Days::new(30),
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

        store
            .record_capacity(
                MintCapacityEvidenceCommand {
                    operator_id: String::from("operator-a"),
                    request: MintCapacityEvidenceRequest {
                        existing_exposure_sat: String::from("1000000"),
                        exposure_limit_sat: String::from("40000000"),
                        source_reference: String::from("mint-book-2026-08"),
                        valid_through: now.date_naive() + chrono::Days::new(30),
                        written_basis: String::from(
                            "Reconciled against the current Mint exposure ledger.",
                        ),
                    },
                },
                now,
            )
            .await
            .unwrap();

        let snapshot = store.for_quote(&quote).await.unwrap();
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["schemaVersion"], "mint-credit-evidence-v1");
        assert_eq!(
            json["acceptorRisk"]["schemaVersion"],
            "mint-acceptor-risk-evidence-v1"
        );
        assert_eq!(json["mintCapacity"]["existingExposureSat"], "1000000");
        assert!(json.get("acceptor_risk").is_none());
        assert_eq!(
            snapshot.acceptor_risk.unwrap().probability_of_default_bps,
            600
        );
        assert_eq!(
            snapshot.mint_capacity.unwrap().existing_exposure_sat,
            "1000000"
        );
    }
}
