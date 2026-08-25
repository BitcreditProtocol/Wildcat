// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
// ----- local modules
pub mod inmemory;
pub mod sqlx;
pub mod surreal;
// ----- local imports
use crate::{
    error::Result,
    quotes::{LightQuote, Quote, Status},
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{quotes, service, TStamp};
    use bcr_common::{cashu, core_tests, wire_tests::random_identity_public_data};
    use bcr_ebill_core::protocol::blockchain::bill::participant::BillParticipant;
    use bcr_wdc_utils::{convert, keys::test_utils as keys_test, surreal as surreal_config};
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
            submitted: TStamp::default(),
            status: quotes::Status::Pending {
                wallet_pubkey: keys_test::publics()[0],
            },
            credit_program: Some(quotes::test_credit_program_binding()),
            authorization_receipt: None,
        }
    }

    fn offered_status(quote: &quotes::Quote) -> quotes::Status {
        quotes::Status::Offered {
            keyset_id: core_tests::generate_random_ecash_keyset().0.id,
            ttl: TStamp::default(),
            discounted: quote.bill.sum,
            wallet_pubkey: keys_test::publics()[0],
        }
    }

    fn accepted_status() -> quotes::Status {
        quotes::Status::Accepted {
            keyset_id: core_tests::generate_random_ecash_keyset().0.id,
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
    async fn authorization_transition_is_atomic_and_idempotent_sqlx(pool: ::sqlx::PgPool) {
        authorization_transition_is_atomic_and_idempotent_for(sqlx::DBQuotes::from_pool(pool))
            .await;
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
            tstamp: TStamp::default(),
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
        update_status_if_offered_ok(db).await;
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
            tstamp: TStamp::from_timestamp(10000, 0).unwrap(),
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
            keyset_id: core_tests::generate_random_ecash_keyset().0.id,
            discounted: bitcoin::Amount::default(),
            wallet_pubkey: keys_test::publics()[0],
        };
        db.store(quote.clone()).await.unwrap();
        let res = db
            .update_status_if_failedebillvalidation(
                quote.id,
                quotes::Status::MintingEnabled {
                    keyset_id: core_tests::generate_random_ecash_keyset().0.id,
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
            tstamp: TStamp::from_timestamp(10000, 0).unwrap(),
        };
        db.store(quote.clone()).await.unwrap();
        let res = db
            .update_status_if_failedebillvalidation(
                quote.id,
                quotes::Status::MintingEnabled {
                    keyset_id: core_tests::generate_random_ecash_keyset().0.id,
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
        let quote = quotes::Quote {
            id: Uuid::new_v4(),
            status: quotes::Status::Pending {
                wallet_pubkey: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                drawee: convert::billidentparticipant_wire2ebill(random_identity_public_data().1)
                    .unwrap(),
                drawer: convert::billidentparticipant_wire2ebill(random_identity_public_data().1)
                    .unwrap(),
                payee: BillParticipant::Ident(
                    convert::billidentparticipant_wire2ebill(random_identity_public_data().1)
                        .unwrap(),
                ),
                endorsees: vec![],
                maturity_date: chrono::NaiveDate::from_ymd_opt(2021, 1, 1).unwrap(),
                ..quotes::BillInfo::random()
            },
            submitted: TStamp::default(),
            credit_program: Some(quotes::test_credit_program_binding()),
            authorization_receipt: None,
        };
        db.store(quote.clone()).await.unwrap();
        let filters = service::ListFilters::default();
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 1);
        let date = chrono::NaiveDate::from_ymd_opt(2021, 1, 1);
        let filters = service::ListFilters {
            bill_maturity_date_from: date,
            ..Default::default()
        };
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 1);
        let date = chrono::NaiveDate::from_ymd_opt(2022, 1, 1);
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
            bill_drawee_id: Some(quote.bill.drawee.node_id),
            ..Default::default()
        };
        let res = db.list_light(filters, None).await.unwrap();
        assert_eq!(res.len(), 1);
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
                maturity_date: chrono::NaiveDate::from_ymd_opt(2021, 1, 1).unwrap(),
                ..quotes::BillInfo::random()
            },
            submitted: TStamp::from_timestamp(100000, 0).unwrap(),
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
                maturity_date: chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
                ..quotes::BillInfo::random()
            },
            submitted: TStamp::from_timestamp(300000, 0).unwrap(),
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
                maturity_date: chrono::NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(),
                ..quotes::BillInfo::random()
            },
            submitted: TStamp::from_timestamp(200000, 0).unwrap(),
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
        let current_holder = BillParticipant::Ident(
            convert::billidentparticipant_wire2ebill(random_identity_public_data().1).unwrap(),
        );
        let quote = quotes::Quote {
            id: Uuid::new_v4(),
            status: quotes::Status::Pending {
                wallet_pubkey: keys_test::publics()[0],
            },
            bill: quotes::BillInfo {
                maturity_date: chrono::NaiveDate::from_ymd_opt(2021, 1, 1).unwrap(),
                payee: current_holder.clone(),
                current_holder,
                ..quotes::BillInfo::random()
            },
            submitted: TStamp::default(),
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
}
