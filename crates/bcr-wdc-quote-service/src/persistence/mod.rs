// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_common::{
    core::{BillId, NodeId},
    wire::{bill as wire_bill, contact as wire_contact, identity as wire_identity},
};
// ----- local modules
pub mod inmemory;
pub mod sqlx;
pub mod surreal;
// ----- local imports
use crate::{
    error::{Error, Result},
    quotes::{self, LightQuote, Quote, Status},
    service::{ListFilters, SortOrder},
};

#[derive(Debug, Clone)]
pub struct ExposureReservationInput {
    pub mint_id: String,
    pub amount_sat: u64,
    pub capacity_evidence_id: uuid::Uuid,
    pub existing_exposure_sat: u64,
    pub exposure_limit_sat: u64,
    pub now: chrono::DateTime<chrono::Utc>,
}

// ----- end imports

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository {
    async fn load(&self, id: uuid::Uuid) -> Result<Option<Quote>>;
    async fn update_status_if_pending(&self, id: uuid::Uuid, quote: Status) -> Result<()>;
    async fn execute_authorization(
        &self,
        quote: Quote,
        exposure: ExposureReservationInput,
    ) -> Result<bcr_common::wire::quotes::CreditAuthorizationReceipt>;
    async fn update_status_if_offered(
        &self,
        id: uuid::Uuid,
        quote: Status,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()>;
    async fn release_committed_exposure(
        &self,
        id: uuid::Uuid,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()>;
    async fn update_status_if_accepted(&self, id: uuid::Uuid, quote: Status) -> Result<()>;
    async fn update_status_if_failedebillvalidation(
        &self,
        id: uuid::Uuid,
        quote: Status,
    ) -> Result<()>;
    async fn list_light(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
    ) -> Result<Vec<LightQuote>>;
    async fn search_by_bill(&self, bill: &BillId, endorser: &NodeId) -> Result<Vec<Quote>>;
    async fn store(&self, quote: Quote) -> Result<()>;
}

time::serde::format_description!(pub db_date, Date, "[year]-[month]-[day]");

const CONTACT_TYPE_PERSON: u8 = 0;
const CONTACT_TYPE_COMPANY: u8 = 1;
const CONTACT_TYPE_ANON: u8 = 2;

// newtype over the `serde_repr` discriminant `wire_contact::ContactType` encodes as.
// serde is transparent for newtype structs, so this is still a bare number on disk.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DbContactType(pub u8);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DbPostalAddress {
    pub country: String,
    pub city: String,
    pub zip: Option<String>,
    pub address: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DbBillIdentParticipant {
    #[serde(rename = "type")]
    pub t: DbContactType,
    pub node_id: String,
    pub name: String,
    #[serde(flatten)]
    pub postal_address: DbPostalAddress,
    pub email: Option<String>,
    pub nostr_relays: Vec<url::Url>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DbBillAnonParticipant {
    pub node_id: String,
    pub nostr_relays: Vec<url::Url>,
}

// externally tagged, as `wire_bill::BillParticipant` is: `{"Ident": {...}}` / `{"Anon": {...}}`
// SurrealQL queries navigate through those tags, keep the variant names as they are.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DbBillParticipant {
    Anon(DbBillAnonParticipant),
    Ident(DbBillIdentParticipant),
}

impl DbBillParticipant {
    pub fn node_id(&self) -> &str {
        match self {
            DbBillParticipant::Anon(data) => &data.node_id,
            DbBillParticipant::Ident(data) => &data.node_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DbBillInfo {
    pub id: String,
    pub drawee: DbBillIdentParticipant,
    pub drawer: DbBillIdentParticipant,
    pub payee: DbBillParticipant,
    pub endorsees: Vec<DbBillParticipant>,
    pub current_holder: DbBillParticipant,
    pub sum: u64,
    #[serde(with = "db_date")]
    pub maturity_date: time::Date,
    pub file_urls: Vec<url::Url>,
    pub shared_bill_data: String,
}

// ///////////////////////////////////////////////////////////////////////// DB conversions

impl From<wire_contact::ContactType> for DbContactType {
    fn from(t: wire_contact::ContactType) -> Self {
        let t = match t {
            wire_contact::ContactType::Person => CONTACT_TYPE_PERSON,
            wire_contact::ContactType::Company => CONTACT_TYPE_COMPANY,
            wire_contact::ContactType::Anon => CONTACT_TYPE_ANON,
        };
        Self(t)
    }
}

impl TryFrom<DbContactType> for wire_contact::ContactType {
    type Error = Error;

    fn try_from(t: DbContactType) -> Result<Self> {
        match t.0 {
            CONTACT_TYPE_PERSON => Ok(wire_contact::ContactType::Person),
            CONTACT_TYPE_COMPANY => Ok(wire_contact::ContactType::Company),
            CONTACT_TYPE_ANON => Ok(wire_contact::ContactType::Anon),
            unknown => Err(Error::QuotesRepository(anyhow!(
                "unknown contact type {unknown}"
            ))),
        }
    }
}

impl From<wire_identity::PostalAddress> for DbPostalAddress {
    fn from(address: wire_identity::PostalAddress) -> Self {
        let wire_identity::PostalAddress {
            country,
            city,
            zip,
            address,
        } = address;
        Self {
            country,
            city,
            zip,
            address,
        }
    }
}

impl From<DbPostalAddress> for wire_identity::PostalAddress {
    fn from(address: DbPostalAddress) -> Self {
        let DbPostalAddress {
            country,
            city,
            zip,
            address,
        } = address;
        Self {
            country,
            city,
            zip,
            address,
        }
    }
}

impl From<wire_bill::BillIdentParticipant> for DbBillIdentParticipant {
    fn from(participant: wire_bill::BillIdentParticipant) -> Self {
        let wire_bill::BillIdentParticipant {
            t,
            node_id,
            name,
            postal_address,
            email,
            nostr_relays,
        } = participant;
        Self {
            t: DbContactType::from(t),
            node_id: node_id.to_string(),
            name,
            postal_address: DbPostalAddress::from(postal_address),
            email,
            nostr_relays,
        }
    }
}

impl TryFrom<DbBillIdentParticipant> for wire_bill::BillIdentParticipant {
    type Error = Error;

    fn try_from(participant: DbBillIdentParticipant) -> Result<Self> {
        let DbBillIdentParticipant {
            t,
            node_id,
            name,
            postal_address,
            email,
            nostr_relays,
        } = participant;
        Ok(Self {
            t: wire_contact::ContactType::try_from(t)?,
            node_id: NodeId::from_str(&node_id).map_err(|e| Error::QuotesRepository(anyhow!(e)))?,
            name,
            postal_address: wire_identity::PostalAddress::from(postal_address),
            email,
            nostr_relays,
        })
    }
}

impl From<wire_bill::BillAnonParticipant> for DbBillAnonParticipant {
    fn from(participant: wire_bill::BillAnonParticipant) -> Self {
        let wire_bill::BillAnonParticipant {
            node_id,
            nostr_relays,
        } = participant;
        Self {
            node_id: node_id.to_string(),
            nostr_relays,
        }
    }
}

impl TryFrom<DbBillAnonParticipant> for wire_bill::BillAnonParticipant {
    type Error = Error;

    fn try_from(participant: DbBillAnonParticipant) -> Result<Self> {
        let DbBillAnonParticipant {
            node_id,
            nostr_relays,
        } = participant;
        Ok(Self {
            node_id: NodeId::from_str(&node_id).map_err(|e| Error::QuotesRepository(anyhow!(e)))?,
            nostr_relays,
        })
    }
}

impl From<wire_bill::BillParticipant> for DbBillParticipant {
    fn from(participant: wire_bill::BillParticipant) -> Self {
        match participant {
            wire_bill::BillParticipant::Anon(data) => Self::Anon(DbBillAnonParticipant::from(data)),
            wire_bill::BillParticipant::Ident(data) => {
                Self::Ident(DbBillIdentParticipant::from(data))
            }
        }
    }
}

impl TryFrom<DbBillParticipant> for wire_bill::BillParticipant {
    type Error = Error;

    fn try_from(participant: DbBillParticipant) -> Result<Self> {
        let participant = match participant {
            DbBillParticipant::Anon(data) => {
                Self::Anon(wire_bill::BillAnonParticipant::try_from(data)?)
            }
            DbBillParticipant::Ident(data) => {
                Self::Ident(wire_bill::BillIdentParticipant::try_from(data)?)
            }
        };
        Ok(participant)
    }
}

impl From<quotes::BillInfo> for DbBillInfo {
    fn from(bill: quotes::BillInfo) -> Self {
        let quotes::BillInfo {
            id,
            drawee,
            drawer,
            payee,
            endorsees,
            current_holder,
            sum,
            maturity_date,
            file_urls,
            shared_bill_data,
        } = bill;
        Self {
            id: id.to_string(),
            drawee: DbBillIdentParticipant::from(drawee),
            drawer: DbBillIdentParticipant::from(drawer),
            payee: DbBillParticipant::from(payee),
            endorsees: endorsees.into_iter().map(DbBillParticipant::from).collect(),
            current_holder: DbBillParticipant::from(current_holder),
            sum: sum.to_sat(),
            maturity_date,
            file_urls,
            shared_bill_data,
        }
    }
}

impl TryFrom<DbBillInfo> for quotes::BillInfo {
    type Error = Error;

    fn try_from(bill: DbBillInfo) -> Result<Self> {
        let DbBillInfo {
            id,
            drawee,
            drawer,
            payee,
            endorsees,
            current_holder,
            sum,
            maturity_date,
            file_urls,
            shared_bill_data,
        } = bill;
        Ok(Self {
            id: BillId::from_str(&id).map_err(|e| Error::QuotesRepository(anyhow!(e)))?,
            drawee: wire_bill::BillIdentParticipant::try_from(drawee)?,
            drawer: wire_bill::BillIdentParticipant::try_from(drawer)?,
            payee: wire_bill::BillParticipant::try_from(payee)?,
            endorsees: endorsees
                .into_iter()
                .map(wire_bill::BillParticipant::try_from)
                .collect::<Result<Vec<_>>>()?,
            current_holder: wire_bill::BillParticipant::try_from(current_holder)?,
            sum: bitcoin::Amount::from_sat(sum),
            maturity_date,
            file_urls,
            shared_bill_data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{quotes, service, TStamp};
    use bcr_common::{
        cashu, core_tests, wire::bill as wire_bill, wire_tests::random_identity_public_data,
    };
    use bcr_wdc_utils::{keys::test_utils as keys_test, surreal as surreal_config};
    use uuid::Uuid;

    async fn init_surreal_db() -> impl Repository {
        surreal::DBQuotes::new(surreal_config::DBConnConfig {
            connection: "mem://".to_string(),
            namespace: "test".to_string(),
            database: "test".to_string(),
        })
        .await
        .unwrap()
    }

    fn init_inmemory_db() -> impl Repository {
        inmemory::QuotesIDMap::default()
    }

    fn pending_quote() -> quotes::Quote {
        quotes::Quote {
            bill: quotes::BillInfo::random(),
            id: Uuid::new_v4(),
            submitted: TStamp::UNIX_EPOCH,
            status: quotes::Status::Pending {
                wallet_pubkey: keys_test::publics()[0],
            },
            credit_program: Some(quotes::test_credit_program_binding()),
            authorization_receipt: None,
        }
    }

    fn offered_status(quote: &quotes::Quote) -> quotes::Status {
        quotes::Status::Offered {
            keyset_id: core_tests::generate_random_ecash_keyset().0.id.into(),
            ttl: TStamp::UNIX_EPOCH,
            discounted: quote.bill.sum,
            wallet_pubkey: keys_test::publics()[0],
        }
    }

    fn accepted_status() -> quotes::Status {
        quotes::Status::Accepted {
            keyset_id: core_tests::generate_random_ecash_keyset().0.id.into(),
            discounted: bitcoin::Amount::default(),
            wallet_pubkey: keys_test::publics()[0],
        }
    }

    #[tokio::test]
    async fn test_store_load_preserves_credit_program() {
        store_load_preserves_credit_program(init_inmemory_db()).await;
        store_load_preserves_credit_program(init_surreal_db().await).await;
    }

    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_store_load_preserves_credit_program_sqlx(pool: ::sqlx::PgPool) {
        store_load_preserves_credit_program(sqlx::DBQuotes::from_pool(pool)).await;
    }

    async fn store_load_preserves_credit_program(db: impl Repository) {
        let quote = pending_quote();
        let expected = quote.credit_program().cloned();
        db.store(quote.clone()).await.unwrap();

        let stored = db.load(quote.id).await.unwrap().unwrap();

        assert_eq!(stored.credit_program(), expected.as_ref());
    }

    #[tokio::test]
    async fn test_store_rejects_unbound_quote() {
        store_rejects_unbound_quote(init_inmemory_db()).await;
        store_rejects_unbound_quote(init_surreal_db().await).await;
    }

    async fn store_rejects_unbound_quote(db: impl Repository) {
        let mut quote = pending_quote();
        quote.credit_program = None;

        assert!(matches!(
            db.store(quote.clone()).await,
            Err(crate::error::Error::CreditProgramNotBound(id)) if id == quote.id
        ));
    }

    fn authorization_receipt(
        operation_id: &str,
    ) -> bcr_common::wire::quotes::CreditAuthorizationReceipt {
        bcr_common::wire::quotes::CreditAuthorizationReceipt {
            receipt_version: String::from("credit-authorization-receipt-v1"),
            operation_id: operation_id.to_owned(),
            authorization_digest: format!("sha256:{}", "a".repeat(64)),
            case_id: String::from("case-a"),
            status: String::from("completed"),
            mint_id: String::from("local-wildcat"),
            bill_id: String::from("bill-a"),
            action: String::from("request_to_mint"),
            effect_id: String::from("effect-a"),
            result_digest: format!("sha256:{}", "b".repeat(64)),
            completed_at: String::from("2026-08-10T12:06:00.000Z"),
            synthetic: true,
        }
    }

    fn exposure(now: TStamp) -> ExposureReservationInput {
        ExposureReservationInput {
            mint_id: String::from("local-wildcat"),
            amount_sat: 8_000_000,
            capacity_evidence_id: uuid::Uuid::nil(),
            existing_exposure_sat: 0,
            exposure_limit_sat: 40_000_000,
            now,
        }
    }

    #[tokio::test]
    async fn authorization_transition_is_atomic_and_idempotent() {
        authorization_transition_is_atomic_and_idempotent_for(init_inmemory_db()).await;
        authorization_transition_is_atomic_and_idempotent_for(init_surreal_db().await).await;
    }

    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn authorization_transition_is_fail_closed_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBQuotes::from_pool(pool);
        let mut quote = pending_quote();
        db.store(quote.clone()).await.unwrap();
        quote.status = offered_status(&quote);
        quote.authorization_receipt = Some(authorization_receipt("sqlx-operation"));

        assert!(matches!(
            db.execute_authorization(quote, exposure(TStamp::default()))
                .await,
            Err(crate::error::Error::CreditCapacityUnavailable)
        ));
    }

    async fn authorization_transition_is_atomic_and_idempotent_for(db: impl Repository) {
        let mut quote = pending_quote();
        db.store(quote.clone()).await.unwrap();
        quote.status = offered_status(&quote);
        let receipt = authorization_receipt(&format!("sha256:{}", "c".repeat(64)));
        quote.authorization_receipt = Some(receipt.clone());

        assert_eq!(
            db.execute_authorization(quote.clone(), exposure(TStamp::default()))
                .await
                .unwrap(),
            receipt
        );
        assert_eq!(
            db.execute_authorization(quote.clone(), exposure(TStamp::default()))
                .await
                .unwrap(),
            receipt
        );

        quote.authorization_receipt =
            Some(authorization_receipt(&format!("sha256:{}", "d".repeat(64))));
        assert!(matches!(
            db.execute_authorization(quote, exposure(TStamp::default()))
                .await,
            Err(crate::error::Error::CreditAuthorizationConflict)
        ));
    }

    #[tokio::test]
    async fn exposure_capacity_is_reserved_atomically() {
        exposure_capacity_is_reserved_atomically_for(init_inmemory_db()).await;
        exposure_capacity_is_reserved_atomically_for(init_surreal_db().await).await;
    }

    async fn exposure_capacity_is_reserved_atomically_for(db: impl Repository) {
        let mut first = pending_quote();
        let mut second = pending_quote();
        db.store(first.clone()).await.unwrap();
        db.store(second.clone()).await.unwrap();
        first.status = offered_status(&first);
        second.status = offered_status(&second);
        first.authorization_receipt =
            Some(authorization_receipt(&format!("sha256:{}", "c".repeat(64))));
        second.authorization_receipt =
            Some(authorization_receipt(&format!("sha256:{}", "d".repeat(64))));
        let mut first_exposure = exposure(TStamp::default());
        first_exposure.exposure_limit_sat = 12_000_000;
        let mut second_exposure = exposure(TStamp::default());
        second_exposure.exposure_limit_sat = 12_000_000;

        let (first_result, second_result) = tokio::join!(
            db.execute_authorization(first, first_exposure),
            db.execute_authorization(second, second_exposure),
        );
        assert_eq!(
            usize::from(first_result.is_ok()) + usize::from(second_result.is_ok()),
            1
        );
        assert!(matches!(
            first_result.as_ref().err().or(second_result.as_ref().err()),
            Some(crate::error::Error::CreditCapacityExceeded)
                | Some(crate::error::Error::QuotesRepository(_))
        ));
    }

    #[tokio::test]
    async fn exposure_is_released_on_rejection_and_committed_on_acceptance() {
        exposure_lifecycle_for(init_inmemory_db()).await;
        exposure_lifecycle_for(init_surreal_db().await).await;
    }

    async fn exposure_lifecycle_for(db: impl Repository) {
        let mut quotes = [pending_quote(), pending_quote(), pending_quote()];
        for quote in &quotes {
            db.store(quote.clone()).await.unwrap();
        }
        for (index, quote) in quotes.iter_mut().enumerate() {
            quote.status = offered_status(quote);
            quote.authorization_receipt =
                Some(authorization_receipt(&format!("sha256:{:064x}", index + 1)));
        }
        let capacity = || ExposureReservationInput {
            exposure_limit_sat: 8_000_000,
            ..exposure(TStamp::default())
        };

        db.execute_authorization(quotes[0].clone(), capacity())
            .await
            .unwrap();
        assert!(matches!(
            db.execute_authorization(quotes[1].clone(), capacity())
                .await,
            Err(crate::error::Error::CreditCapacityExceeded)
        ));
        db.update_status_if_offered(
            quotes[0].id,
            quotes::Status::Rejected {
                tstamp: TStamp::default(),
                discounted: bitcoin::Amount::from_sat(8_000_000),
            },
            TStamp::default(),
        )
        .await
        .unwrap();
        db.execute_authorization(quotes[1].clone(), capacity())
            .await
            .unwrap();
        db.update_status_if_offered(quotes[1].id, accepted_status(), TStamp::default())
            .await
            .unwrap();
        assert!(matches!(
            db.execute_authorization(quotes[2].clone(), capacity())
                .await,
            Err(crate::error::Error::CreditCapacityExceeded)
        ));
        db.release_committed_exposure(quotes[1].id, TStamp::default())
            .await
            .unwrap();
        db.release_committed_exposure(quotes[1].id, TStamp::default())
            .await
            .unwrap();
        db.execute_authorization(quotes[2].clone(), capacity())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_update_status_if_pending_ok() {
        let db = init_inmemory_db();
        update_status_if_pending_ok(db).await;
        let db = init_surreal_db().await;
        update_status_if_pending_ok(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_update_status_if_pending_ok_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBQuotes::from_pool(pool);
        update_status_if_pending_ok(db).await;
    }
    async fn update_status_if_pending_ok(db: impl Repository) {
        let quote = pending_quote();
        db.store(quote.clone()).await.unwrap();
        let res = db
            .update_status_if_pending(quote.id, offered_status(&quote))
            .await;
        assert!(res.is_ok());
        let updated = db.load(quote.id).await.unwrap().unwrap();
        assert!(matches!(updated.status, quotes::Status::Offered { .. }));
    }

    #[tokio::test]
    async fn test_update_status_if_pending_ko() {
        let db = init_inmemory_db();
        update_status_if_pending_ko(db).await;
        let db = init_surreal_db().await;
        update_status_if_pending_ko(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_update_status_if_pending_ko_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBQuotes::from_pool(pool);
        update_status_if_pending_ko(db).await;
    }
    async fn update_status_if_pending_ko(db: impl Repository) {
        let mut quote = pending_quote();
        quote.status = quotes::Status::Rejected {
            tstamp: TStamp::UNIX_EPOCH,
            discounted: bitcoin::Amount::default(),
        };
        db.store(quote.clone()).await.unwrap();
        let res = db
            .update_status_if_pending(quote.id, offered_status(&quote))
            .await;
        assert!(res.is_err());
        let content = db.load(quote.id).await.unwrap().unwrap();
        assert!(matches!(content.status, quotes::Status::Rejected { .. }));
    }

    #[tokio::test]
    async fn test_update_status_if_offered_ok() {
        let db = init_inmemory_db();
        update_status_if_offered_ok(db).await;
        let db = init_surreal_db().await;
        update_status_if_offered_ok(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_update_status_if_offered_ok_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBQuotes::from_pool(pool);
        let quote = pending_quote();
        db.store(quote.clone()).await.unwrap();
        db.update_status_if_pending(quote.id, offered_status(&quote))
            .await
            .unwrap();

        db.update_status_if_offered(quote.id, accepted_status(), TStamp::default())
            .await
            .unwrap();

        let updated = db.load(quote.id).await.unwrap().unwrap();
        assert!(matches!(updated.status, quotes::Status::Accepted { .. }));
    }
    async fn update_status_if_offered_ok(db: impl Repository) {
        let mut quote = pending_quote();
        db.store(quote.clone()).await.unwrap();
        quote.status = offered_status(&quote);
        quote.authorization_receipt =
            Some(authorization_receipt(&format!("sha256:{}", "c".repeat(64))));
        db.execute_authorization(quote.clone(), exposure(TStamp::default()))
            .await
            .unwrap();
        let res = db
            .update_status_if_offered(quote.id, accepted_status(), TStamp::default())
            .await;
        assert!(res.is_ok());
        let updated = db.load(quote.id).await.unwrap().unwrap();
        assert!(matches!(updated.status, quotes::Status::Accepted { .. }));
    }

    #[tokio::test]
    async fn test_update_status_if_offered_ko() {
        let db = init_inmemory_db();
        update_status_if_offered_ko(db).await;
        let db = init_surreal_db().await;
        update_status_if_offered_ko(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_update_status_if_offered_ko_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBQuotes::from_pool(pool);
        update_status_if_offered_ko(db).await;
    }
    async fn update_status_if_offered_ko(db: impl Repository) {
        let mut quote = pending_quote();
        quote.status = quotes::Status::Denied {
            tstamp: TStamp::from_unix_timestamp(10000).unwrap(),
        };
        db.store(quote.clone()).await.unwrap();
        let res = db
            .update_status_if_offered(quote.id, offered_status(&quote), TStamp::default())
            .await;
        assert!(res.is_err());
        let content = db.load(quote.id).await.unwrap().unwrap();
        assert!(matches!(content.status, quotes::Status::Denied { .. }));
    }

    #[tokio::test]
    async fn test_update_status_if_failedebillvalidation_ok() {
        let db = init_inmemory_db();
        update_status_if_failedebillvalidation_ok(db).await;
        let db = init_surreal_db().await;
        update_status_if_failedebillvalidation_ok(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_update_status_if_failedebillvalidation_ok_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBQuotes::from_pool(pool);
        update_status_if_failedebillvalidation_ok(db).await;
    }
    async fn update_status_if_failedebillvalidation_ok(db: impl Repository) {
        let mut quote = pending_quote();
        quote.status = quotes::Status::FailedEbillValidation {
            keyset_id: core_tests::generate_random_ecash_keyset().0.id.into(),
            discounted: bitcoin::Amount::default(),
            wallet_pubkey: keys_test::publics()[0],
        };
        db.store(quote.clone()).await.unwrap();
        let res = db
            .update_status_if_failedebillvalidation(
                quote.id,
                quotes::Status::MintingEnabled {
                    keyset_id: core_tests::generate_random_ecash_keyset().0.id.into(),
                    discounted: bitcoin::Amount::default(),
                    wallet_pubkey: keys_test::publics()[0],
                    fee: cashu::Amount::from(10),
                },
            )
            .await;
        assert!(res.is_ok());
        let updated = db.load(quote.id).await.unwrap().unwrap();
        assert!(matches!(
            updated.status,
            quotes::Status::MintingEnabled { .. }
        ));
    }

    #[tokio::test]
    async fn test_update_status_if_failedebillvalidation_ko() {
        let db = init_inmemory_db();
        update_status_if_failedebillvalidation_ko(db).await;
        let db = init_surreal_db().await;
        update_status_if_failedebillvalidation_ko(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_update_status_if_failedebillvalidation_ko_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBQuotes::from_pool(pool);
        update_status_if_failedebillvalidation_ko(db).await;
    }
    async fn update_status_if_failedebillvalidation_ko(db: impl Repository) {
        let mut quote = pending_quote();
        quote.status = quotes::Status::Denied {
            tstamp: TStamp::from_unix_timestamp(10000).unwrap(),
        };
        db.store(quote.clone()).await.unwrap();
        let res = db
            .update_status_if_failedebillvalidation(
                quote.id,
                quotes::Status::MintingEnabled {
                    keyset_id: core_tests::generate_random_ecash_keyset().0.id.into(),
                    discounted: quote.bill.sum,
                    wallet_pubkey: keys_test::publics()[0],
                    fee: cashu::Amount::from(10),
                },
            )
            .await;
        assert!(res.is_err());
        let content = db.load(quote.id).await.unwrap().unwrap();
        assert!(matches!(content.status, quotes::Status::Denied { .. }));
    }

    #[tokio::test]
    async fn test_list_light_filter() {
        let db = init_inmemory_db();
        list_light_filter(db).await;
        let db = init_surreal_db().await;
        list_light_filter(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_list_light_filter_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBQuotes::from_pool(pool);
        list_light_filter(db).await;
    }
    async fn list_light_filter(db: impl Repository) {
        let holder = wire_bill::BillParticipant::Ident(random_identity_public_data().1);
        let quote = quotes::Quote {
            id: Uuid::new_v4(),
            status: quotes::Status::Pending {
                wallet_pubkey: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                drawee: random_identity_public_data().1,
                drawer: random_identity_public_data().1,
                payee: holder.clone(),
                current_holder: holder.clone(),
                endorsees: vec![],
                maturity_date: time::Date::from_calendar_date(
                    2021,
                    time::Month::try_from(1u8).unwrap(),
                    1,
                )
                .unwrap(),
                ..quotes::BillInfo::random()
            },
            submitted: TStamp::UNIX_EPOCH,
            credit_program: Some(quotes::test_credit_program_binding()),
            authorization_receipt: None,
        };
        db.store(quote.clone()).await.unwrap();
        let filters = service::ListFilters::default();
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 1);
        let date =
            time::Date::from_calendar_date(2021, time::Month::try_from(1u8).unwrap(), 1).ok();
        let filters = service::ListFilters {
            bill_maturity_date_from: date,
            ..Default::default()
        };
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 1);
        let date =
            time::Date::from_calendar_date(2022, time::Month::try_from(1u8).unwrap(), 1).ok();
        let filters = service::ListFilters {
            bill_maturity_date_from: date,
            ..Default::default()
        };
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 0);
        let filters = service::ListFilters {
            status: Some(quotes::StatusDiscriminants::Pending),
            bill_drawee_id: Some(random_identity_public_data().1.node_id),
            ..Default::default()
        };
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 0);
        let filters = service::ListFilters {
            status: Some(quotes::StatusDiscriminants::Pending),
            bill_drawee_id: Some(quote.bill.drawee.node_id.clone()),
            ..Default::default()
        };
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 1);
        let filters = service::ListFilters {
            bill_payer_id: Some(quote.bill.payee.node_id()),
            ..Default::default()
        };
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 1);
        let filters = service::ListFilters {
            bill_payer_id: Some(random_identity_public_data().1.node_id),
            ..Default::default()
        };
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 0);
        let filters = service::ListFilters {
            bill_holder_id: Some(quote.bill.current_holder.node_id()),
            ..Default::default()
        };
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 1);
        let filters = service::ListFilters {
            bill_holder_id: Some(random_identity_public_data().1.node_id),
            ..Default::default()
        };
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 0);
    }

    #[tokio::test]
    async fn test_list_light_sort() {
        let db = init_inmemory_db();
        list_light_sort(db).await;
        let db = init_surreal_db().await;
        list_light_sort(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_list_light_sort_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBQuotes::from_pool(pool);
        list_light_sort(db).await;
    }
    async fn list_light_sort(db: impl Repository) {
        let qid1 = Uuid::new_v4();
        let quote = quotes::Quote {
            id: qid1,
            status: quotes::Status::Pending {
                wallet_pubkey: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                maturity_date: time::Date::from_calendar_date(
                    2021,
                    time::Month::try_from(1u8).unwrap(),
                    1,
                )
                .unwrap(),
                ..quotes::BillInfo::random()
            },
            submitted: TStamp::from_unix_timestamp(100000).unwrap(),
            credit_program: Some(quotes::test_credit_program_binding()),
            authorization_receipt: None,
        };
        db.store(quote).await.unwrap();
        let qid2 = Uuid::new_v4();
        let quote = quotes::Quote {
            id: qid2,
            status: quotes::Status::Pending {
                wallet_pubkey: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                maturity_date: time::Date::from_calendar_date(
                    2020,
                    time::Month::try_from(1u8).unwrap(),
                    1,
                )
                .unwrap(),
                ..quotes::BillInfo::random()
            },
            submitted: TStamp::from_unix_timestamp(300000).unwrap(),
            credit_program: Some(quotes::test_credit_program_binding()),
            authorization_receipt: None,
        };
        db.store(quote).await.unwrap();
        let qid3 = Uuid::new_v4();
        let quote = quotes::Quote {
            id: qid3,
            status: quotes::Status::Pending {
                wallet_pubkey: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                maturity_date: time::Date::from_calendar_date(
                    2022,
                    time::Month::try_from(1u8).unwrap(),
                    1,
                )
                .unwrap(),
                ..quotes::BillInfo::random()
            },
            submitted: TStamp::from_unix_timestamp(200000).unwrap(),
            credit_program: Some(quotes::test_credit_program_binding()),
            authorization_receipt: None,
        };
        db.store(quote).await.unwrap();
        let filters = service::ListFilters::default();
        let res = db
            .list_light(filters, Some(SortOrder::BillMaturityDateAsc))
            .await
            .unwrap();
        assert_eq!(res.len(), 3);
        assert_eq!(res[0].id, qid2);
        assert_eq!(res[1].id, qid1);
        assert_eq!(res[2].id, qid3);
        let filters = service::ListFilters::default();
        let res = db
            .list_light(filters, Some(SortOrder::BillMaturityDateDesc))
            .await
            .unwrap();
        assert_eq!(res.len(), 3);
        assert_eq!(res[0].id, qid3);
        assert_eq!(res[1].id, qid1);
        assert_eq!(res[2].id, qid2);
        let filters = service::ListFilters::default();
        let res = db
            .list_light(filters, Some(SortOrder::SubmittedAsc))
            .await
            .unwrap();
        assert_eq!(res.len(), 3);
        assert_eq!(res[0].id, qid1);
        assert_eq!(res[1].id, qid3);
        assert_eq!(res[2].id, qid2);
        let filters = service::ListFilters::default();
        let res = db
            .list_light(filters, Some(SortOrder::SubmittedDesc))
            .await
            .unwrap();
        assert_eq!(res.len(), 3);
        assert_eq!(res[0].id, qid2);
        assert_eq!(res[1].id, qid3);
        assert_eq!(res[2].id, qid1);
    }

    #[tokio::test]
    async fn test_search_by_bill() {
        let db = init_inmemory_db();
        search_by_bill(db).await;
        let db = init_surreal_db().await;
        search_by_bill(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_search_by_bill_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBQuotes::from_pool(pool);
        search_by_bill(db).await;
    }
    async fn search_by_bill(db: impl Repository) {
        let current_holder = wire_bill::BillParticipant::Ident(random_identity_public_data().1);
        let quote = quotes::Quote {
            id: Uuid::new_v4(),
            status: quotes::Status::Pending {
                wallet_pubkey: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                maturity_date: time::Date::from_calendar_date(
                    2021,
                    time::Month::try_from(1u8).unwrap(),
                    1,
                )
                .unwrap(),
                payee: current_holder.clone(),
                current_holder,
                ..quotes::BillInfo::random()
            },
            submitted: TStamp::UNIX_EPOCH,
            credit_program: Some(quotes::test_credit_program_binding()),
            authorization_receipt: None,
        };
        db.store(quote.clone()).await.unwrap();
        let result = db
            .search_by_bill(&quote.bill.id, &quote.bill.current_holder.node_id())
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
    }

    // ----- DB types: the on-disk encoding is frozen, these tests are the tripwire

    fn full_billinfo() -> quotes::BillInfo {
        let mut drawer = random_identity_public_data().1;
        drawer.t = wire_contact::ContactType::Company;
        drawer.postal_address.zip = Some(String::from("1010"));
        drawer.email = Some(String::from("drawer@example.com"));
        drawer.nostr_relays = vec![url::Url::parse("wss://relay.example.com/").unwrap()];
        let holder = wire_bill::BillParticipant::Anon(wire_bill::BillAnonParticipant {
            node_id: random_identity_public_data().1.node_id,
            nostr_relays: vec![url::Url::parse("wss://relay.example.org/").unwrap()],
        });
        quotes::BillInfo {
            id: core_tests::random_bill_id(),
            drawee: random_identity_public_data().1,
            drawer,
            payee: wire_bill::BillParticipant::Ident(random_identity_public_data().1),
            endorsees: vec![holder.clone()],
            current_holder: holder,
            sum: bitcoin::Amount::from_sat(1000),
            maturity_date: time::macros::date!(2021 - 01 - 01),
            file_urls: vec![url::Url::parse("https://example.com/f.pdf").unwrap()],
            shared_bill_data: String::from("sharedbilldata"),
        }
    }

    fn all_statuses() -> Vec<quotes::Status> {
        let keyset_id = core_tests::generate_random_ecash_keyset().0.id;
        let wallet_pubkey = keys_test::publics()[0];
        let discounted = bitcoin::Amount::from_sat(42);
        let tstamp = TStamp::from_unix_timestamp(1_600_000_000).unwrap();
        vec![
            quotes::Status::Pending { wallet_pubkey },
            quotes::Status::Canceled { tstamp },
            quotes::Status::Denied { tstamp },
            quotes::Status::Offered {
                keyset_id: keyset_id.into(),
                ttl: tstamp,
                discounted,
                wallet_pubkey,
            },
            quotes::Status::OfferExpired { discounted, tstamp },
            quotes::Status::Rejected { discounted, tstamp },
            quotes::Status::Accepted {
                discounted,
                keyset_id: keyset_id.into(),
                wallet_pubkey,
            },
            quotes::Status::MintingEnabled {
                keyset_id: keyset_id.into(),
                wallet_pubkey,
                discounted,
                fee: cashu::Amount::from(10),
            },
            quotes::Status::FailedEbillValidation {
                discounted,
                keyset_id: keyset_id.into(),
                wallet_pubkey,
            },
        ]
    }

    #[test]
    fn db_billinfo_encoding_is_frozen() {
        let bill = full_billinfo();
        let encoded = serde_json::to_value(DbBillInfo::from(bill.clone())).unwrap();

        assert_eq!(encoded["id"], serde_json::json!(bill.id.to_string()));
        assert_eq!(encoded["sum"], serde_json::json!(1000));
        assert_eq!(encoded["maturity_date"], serde_json::json!("2021-01-01"));
        assert_eq!(
            encoded["file_urls"],
            serde_json::json!(["https://example.com/f.pdf"])
        );
        assert_eq!(
            encoded["shared_bill_data"],
            serde_json::json!("sharedbilldata")
        );

        // the contact type is a number under the key `type`, not a string under `t`
        assert_eq!(encoded["drawer"]["type"], serde_json::json!(1));
        // the postal address is flattened onto the participant
        assert!(encoded["drawer"].get("postal_address").is_none());
        assert_eq!(
            encoded["drawer"]["country"],
            serde_json::json!(bill.drawer.postal_address.country)
        );
        assert_eq!(encoded["drawer"]["zip"], serde_json::json!("1010"));
        assert_eq!(
            encoded["drawer"]["email"],
            serde_json::json!("drawer@example.com")
        );
        assert_eq!(
            encoded["drawer"]["node_id"],
            serde_json::json!(bill.drawer.node_id.to_string())
        );
        assert_eq!(
            encoded["drawer"]["nostr_relays"],
            serde_json::json!(["wss://relay.example.com/"])
        );

        // participants are externally tagged, SurrealQL navigates through those tags
        assert!(encoded["payee"].get("Ident").is_some());
        assert!(encoded["current_holder"].get("Anon").is_some());
        assert!(encoded["endorsees"][0].get("Anon").is_some());
    }

    // `quotes::Status` is stored as-is, but the SurrealQL `status.status` comparisons and
    // the postgres `jsonb_set(blob, '{data,status}')` update depend on how it encodes.
    #[test]
    fn stored_status_encoding_is_frozen() {
        let keyset_id = core_tests::generate_random_ecash_keyset().0.id;
        let wallet_pubkey = keys_test::publics()[0];
        let encoded = serde_json::to_value(quotes::Status::Offered {
            keyset_id: keyset_id.into(),
            ttl: TStamp::UNIX_EPOCH,
            discounted: bitcoin::Amount::from_sat(42),
            wallet_pubkey,
        })
        .unwrap();
        assert_eq!(
            encoded,
            serde_json::json!({
                "status": "Offered",
                "keyset_id": keyset_id.to_string(),
                "ttl": "1970-01-01T00:00:00Z",
                "discounted": 42,
                "wallet_pubkey": wallet_pubkey.to_hex(),
            })
        );

        // every variant carries the internal `status` tag the queries match on
        for status in all_statuses() {
            let encoded = serde_json::to_value(&status).unwrap();
            let discriminant = quotes::StatusDiscriminants::from(status);
            assert_eq!(
                encoded["status"],
                serde_json::to_value(discriminant).unwrap()
            );
        }

        // the surreal `status.status` comparisons bind the serde spelling, the postgres
        // `status` column stores the strum one - they are deliberately different
        assert_eq!(
            quotes::StatusDiscriminants::FailedEbillValidation.to_string(),
            "failedebillvalidation"
        );
        assert_eq!(
            serde_json::to_value(quotes::StatusDiscriminants::FailedEbillValidation).unwrap(),
            serde_json::json!("FailedEbillValidation")
        );
    }

    #[test]
    fn db_billinfo_roundtrip() {
        let bill = full_billinfo();
        let back = quotes::BillInfo::try_from(DbBillInfo::from(bill.clone())).unwrap();
        assert_eq!(bill, back);
        let random = quotes::BillInfo::random();
        let back = quotes::BillInfo::try_from(DbBillInfo::from(random.clone())).unwrap();
        assert_eq!(random, back);
    }

    // The DB types were introduced to freeze what `QuoteBlob::V1` already holds, which
    // today is exactly the `bcr_common` wire encoding. If this test fails the wire types
    // have drifted and the DB types are correctly holding the line: delete this test,
    // do not "fix" the DB types to follow.
    #[test]
    fn db_encoding_matches_v1_wire_encoding() {
        let bill = full_billinfo();
        let wire_encoded = serde_json::to_value(&bill).unwrap();
        // what we write is what the wire types used to write ...
        assert_eq!(
            serde_json::to_value(DbBillInfo::from(bill.clone())).unwrap(),
            wire_encoded
        );
        // ... and what they used to write still reads back into the DB types
        assert_eq!(
            serde_json::from_value::<DbBillInfo>(wire_encoded).unwrap(),
            DbBillInfo::from(bill)
        );
    }
}
