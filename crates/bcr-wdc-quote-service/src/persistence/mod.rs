// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use uuid::Uuid;
// ----- local modules
pub mod inmemory;
pub mod sqlx;
pub mod surreal;
// ----- local imports
use crate::{
    error::Result,
    quotes::{LightQuote, Quote, Status},
    service::{ListFilters, SortOrder},
    TStamp,
};

// ----- end imports

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository {
    async fn load(&self, id: uuid::Uuid) -> Result<Option<Quote>>;
    async fn update_status_if_pending(&self, id: uuid::Uuid, quote: Status) -> Result<()>;
    async fn update_status_if_offered(&self, id: uuid::Uuid, quote: Status) -> Result<()>;
    async fn update_status_if_accepted(&self, id: uuid::Uuid, quote: Status) -> Result<()>;
    async fn update_status_if_failedebillvalidation(
        &self,
        id: uuid::Uuid,
        quote: Status,
    ) -> Result<()>;
    async fn list_pendings(&self, since: Option<TStamp>) -> Result<Vec<Uuid>>;
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
    use crate::{quotes, service};
    use bcr_common::{cashu, core_tests, wire_tests::random_identity_public_data};
    use bcr_ebill_core::protocol::blockchain::bill::participant::BillParticipant;
    use bcr_wdc_utils::{convert, keys::test_utils as keys_test, surreal as surreal_config};

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
        quote.status = offered_status(&quote);
        db.store(quote.clone()).await.unwrap();
        let res = db
            .update_status_if_offered(quote.id, accepted_status())
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
            .update_status_if_offered(quote.id, offered_status(&quote))
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
            submitted: TStamp::default(),
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
            submitted: TStamp::default(),
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
            submitted: TStamp::default(),
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
        };
        db.store(quote.clone()).await.unwrap();
        let result = db
            .search_by_bill(&quote.bill.id, &quote.bill.current_holder.node_id())
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
    }
}
