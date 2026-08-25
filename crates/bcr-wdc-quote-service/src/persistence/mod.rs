// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use bcr_common::wire::quotes::{CreditAuthorizationReceipt, SignedCreditQuoteReissuePermit};
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

#[derive(Debug, Clone)]
pub struct GovernedDenialInput {
    pub quote_id: uuid::Uuid,
    pub receipt: CreditAuthorizationReceipt,
    pub denied_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
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
    async fn execute_governed_denial(
        &self,
        input: GovernedDenialInput,
    ) -> Result<CreditAuthorizationReceipt>;
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
    /// Atomically stores a normal enquiry only while the caller's observed bill/holder head is
    /// still current. A lost race returns the winning quote id instead of creating a duplicate.
    async fn store_if_latest(
        &self,
        expected_latest: Option<uuid::Uuid>,
        quote: Quote,
    ) -> Result<uuid::Uuid>;
    async fn execute_quote_reissue(
        &self,
        signed: SignedCreditQuoteReissuePermit,
        quote: Quote,
        consumed_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<uuid::Uuid>;
}

pub(super) fn same_governed_denial_authority(
    stored: &CreditAuthorizationReceipt,
    requested: &CreditAuthorizationReceipt,
) -> bool {
    stored.receipt_version == requested.receipt_version
        && stored.operation_id == requested.operation_id
        && stored.case_id == requested.case_id
        && stored.status == "completed"
        && requested.status == "completed"
        && stored.mint_id == requested.mint_id
        && stored.bill_id == requested.bill_id
        && stored.action == crate::authorization::QUOTE_DENIAL_ACTION
        && requested.action == crate::authorization::QUOTE_DENIAL_ACTION
        && stored.effect_id == requested.effect_id
        && stored.synthetic
        && requested.synthetic
}

fn same_semantic_quote(stored: &Quote, requested: &Quote) -> bool {
    stored.bill.id == requested.bill.id
        && stored.bill.drawee == requested.bill.drawee
        && stored.bill.drawer == requested.bill.drawer
        && stored.bill.payee == requested.bill.payee
        && stored.bill.endorsees == requested.bill.endorsees
        && stored.bill.current_holder == requested.bill.current_holder
        && stored.bill.sum == requested.bill.sum
        && stored.bill.maturity_date == requested.bill.maturity_date
        && stored.credit_program == requested.credit_program
}

pub(super) fn same_pending_quote_request(stored: &Quote, requested: &Quote) -> bool {
    same_semantic_quote(stored, requested)
        && matches!(
            (&stored.status, &requested.status),
            (
                Status::Pending {
                    wallet_pubkey: stored_wallet,
                },
                Status::Pending {
                    wallet_pubkey: requested_wallet,
                }
            ) if stored_wallet == requested_wallet
        )
}

pub(super) fn same_executed_quote(
    stored: &Quote,
    requested: &Quote,
    minting_pubkey: bcr_common::cashu::PublicKey,
) -> bool {
    stored.id == requested.id
        && same_semantic_quote(stored, requested)
        && matches!(
            &requested.status,
            Status::Pending { wallet_pubkey } if *wallet_pubkey == minting_pubkey
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{quotes, service, TStamp};
    use bcr_common::{cashu, core_tests, wire_tests::random_identity_public_data};
    use bcr_ebill_core::protocol::blockchain::bill::participant::BillParticipant;
    use bcr_wdc_utils::{convert, keys::test_utils as keys_test, surreal as surreal_config};
    use uuid::Uuid;

    async fn init_surreal_db() -> surreal::DBQuotes {
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

    fn authorized_offer_candidate(quote: &quotes::Quote, operation_id: &str) -> quotes::Quote {
        let mut offered = quote.clone();
        offered.status = offered_status(&offered);
        let mut receipt = authorization_receipt(operation_id);
        receipt.bill_id = offered.bill.id.to_string();
        receipt.effect_id = offered.id.to_string();
        let quotes::Status::Offered {
            discounted, ttl, ..
        } = &offered.status
        else {
            unreachable!("test helper always creates an offered quote")
        };
        receipt.result_digest =
            crate::authorization::offer_result_digest(offered.id, *discounted, *ttl);
        offered.authorization_receipt = Some(receipt);
        offered
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

    fn denial_input(
        quote: &quotes::Quote,
        operation_id: &str,
        now: TStamp,
        expires_at: TStamp,
    ) -> GovernedDenialInput {
        let completed_at = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        GovernedDenialInput {
            receipt: bcr_common::wire::quotes::CreditAuthorizationReceipt {
                receipt_version: String::from("credit-authorization-receipt-v1"),
                operation_id: operation_id.to_owned(),
                authorization_digest: format!("sha256:{}", "a".repeat(64)),
                case_id: uuid::Uuid::from_u128(10).to_string(),
                status: String::from("completed"),
                mint_id: String::from("local-wildcat"),
                bill_id: quote.bill.id.to_string(),
                action: String::from(crate::authorization::QUOTE_DENIAL_ACTION),
                effect_id: quote.id.to_string(),
                result_digest: crate::authorization::denial_result_digest(quote.id, &completed_at),
                completed_at,
                synthetic: true,
            },
            quote_id: quote.id,
            denied_at: now,
            expires_at,
        }
    }

    #[tokio::test]
    async fn governed_denial_is_atomic_and_replays_after_expiry() {
        governed_denial_is_atomic_and_replays_after_expiry_for(init_inmemory_db()).await;
        governed_denial_is_atomic_and_replays_after_expiry_for(init_surreal_db().await).await;
    }

    async fn governed_denial_is_atomic_and_replays_after_expiry_for(db: impl Repository) {
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-25T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let quote = pending_quote();
        db.store(quote.clone()).await.unwrap();
        let first = denial_input(
            &quote,
            &format!("sha256:{}", "c".repeat(64)),
            now,
            now + chrono::Duration::hours(1),
        );

        let receipt = db.execute_governed_denial(first.clone()).await.unwrap();
        let mut renewed = denial_input(
            &quote,
            &first.receipt.operation_id,
            now + chrono::Duration::days(2),
            now + chrono::Duration::days(3),
        );
        renewed.receipt.authorization_digest = format!("sha256:{}", "d".repeat(64));
        assert_eq!(db.execute_governed_denial(renewed).await.unwrap(), receipt);
        let stored = db.load(quote.id).await.unwrap().unwrap();
        assert!(matches!(stored.status, quotes::Status::Denied { tstamp } if tstamp == now));
        assert_eq!(stored.authorization_receipt(), Some(&receipt));

        let conflict = denial_input(
            &quote,
            &format!("sha256:{}", "e".repeat(64)),
            now,
            now + chrono::Duration::hours(1),
        );
        assert!(matches!(
            db.execute_governed_denial(conflict).await,
            Err(crate::error::Error::CreditQuoteDenialConflict)
        ));
    }

    #[tokio::test]
    async fn expired_governed_denial_does_not_mutate_quote() {
        expired_governed_denial_does_not_mutate_quote_for(init_inmemory_db()).await;
        expired_governed_denial_does_not_mutate_quote_for(init_surreal_db().await).await;
    }

    async fn expired_governed_denial_does_not_mutate_quote_for(db: impl Repository) {
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-25T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let quote = pending_quote();
        db.store(quote.clone()).await.unwrap();
        let expired = denial_input(&quote, &format!("sha256:{}", "c".repeat(64)), now, now);

        assert!(matches!(
            db.execute_governed_denial(expired).await,
            Err(crate::error::Error::CreditQuoteDenialInvalid)
        ));
        let stored = db.load(quote.id).await.unwrap().unwrap();
        assert!(matches!(stored.status, quotes::Status::Pending { .. }));
        assert!(stored.authorization_receipt().is_none());
    }

    #[tokio::test]
    async fn governed_denial_concurrent_commands_have_one_authority() {
        governed_denial_concurrent_commands_have_one_authority_for(init_inmemory_db()).await;
        governed_denial_concurrent_commands_have_one_authority_for(init_surreal_db().await).await;
    }

    async fn governed_denial_concurrent_commands_have_one_authority_for(db: impl Repository) {
        let backend = std::any::type_name_of_val(&db);
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-25T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let quote = pending_quote();
        db.store(quote.clone()).await.unwrap();
        let left = denial_input(
            &quote,
            &format!("sha256:{}", "c".repeat(64)),
            now,
            now + chrono::Duration::hours(1),
        );
        let right = denial_input(
            &quote,
            &format!("sha256:{}", "d".repeat(64)),
            now,
            now + chrono::Duration::hours(1),
        );

        let (left, right) = tokio::join!(
            db.execute_governed_denial(left),
            db.execute_governed_denial(right)
        );
        assert_eq!(
            usize::from(left.is_ok()) + usize::from(right.is_ok()),
            1,
            "{backend} accepted two conflicting denial authorities"
        );
        assert!(matches!(
            left.as_ref().err().or(right.as_ref().err()),
            Some(crate::error::Error::CreditQuoteDenialConflict)
                | Some(crate::error::Error::QuotesRepository(_))
        ));
    }

    #[tokio::test]
    async fn governed_denial_and_offer_have_one_pending_transition_and_exposure() {
        governed_denial_and_offer_have_one_pending_transition_and_exposure_for(init_inmemory_db())
            .await;
        governed_denial_and_offer_have_one_pending_transition_and_exposure_for(
            init_surreal_db().await,
        )
        .await;
    }

    async fn governed_denial_and_offer_have_one_pending_transition_and_exposure_for(
        db: impl Repository,
    ) {
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-25T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let quote = pending_quote();
        let fallback = pending_quote();
        db.store(quote.clone()).await.unwrap();
        db.store(fallback.clone()).await.unwrap();
        let offered = authorized_offer_candidate(&quote, &format!("sha256:{}", "e".repeat(64)));
        let fallback_offer =
            authorized_offer_candidate(&fallback, &format!("sha256:{}", "f".repeat(64)));
        let denial = denial_input(
            &quote,
            &format!("sha256:{}", "d".repeat(64)),
            now,
            now + chrono::Duration::hours(1),
        );
        let capacity = || ExposureReservationInput {
            exposure_limit_sat: 8_000_000,
            ..exposure(now)
        };

        let (offer_result, denial_result) = tokio::join!(
            db.execute_authorization(offered.clone(), capacity()),
            db.execute_governed_denial(denial.clone()),
        );
        assert_eq!(
            usize::from(offer_result.is_ok()) + usize::from(denial_result.is_ok()),
            1
        );
        let stored = db.load(quote.id).await.unwrap().unwrap();
        match (offer_result, denial_result) {
            (Ok(offer_receipt), Err(crate::error::Error::CreditQuoteDenialConflict)) => {
                assert!(matches!(stored.status, quotes::Status::Offered { .. }));
                assert_eq!(stored.authorization_receipt(), Some(&offer_receipt));
                assert!(matches!(
                    db.execute_authorization(fallback_offer, capacity()).await,
                    Err(crate::error::Error::CreditCapacityExceeded)
                ));
            }
            (Err(crate::error::Error::CreditAuthorizationConflict), Ok(denial_receipt)) => {
                assert!(matches!(stored.status, quotes::Status::Denied { .. }));
                assert_eq!(stored.authorization_receipt(), Some(&denial_receipt));
                db.execute_authorization(fallback_offer, capacity())
                    .await
                    .expect("a winning denial must not reserve exposure");
            }
            unexpected => panic!("incoherent offer/denial race result: {unexpected:?}"),
        }
    }

    #[tokio::test]
    async fn governed_denial_and_cancel_have_one_pending_transition_without_exposure() {
        governed_denial_and_cancel_have_one_pending_transition_without_exposure_for(
            init_inmemory_db(),
        )
        .await;
        governed_denial_and_cancel_have_one_pending_transition_without_exposure_for(
            init_surreal_db().await,
        )
        .await;
    }

    async fn governed_denial_and_cancel_have_one_pending_transition_without_exposure_for(
        db: impl Repository,
    ) {
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-25T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let quote = pending_quote();
        let fallback = pending_quote();
        db.store(quote.clone()).await.unwrap();
        db.store(fallback.clone()).await.unwrap();
        let denial = denial_input(
            &quote,
            &format!("sha256:{}", "d".repeat(64)),
            now,
            now + chrono::Duration::hours(1),
        );
        let canceled = quotes::Status::Canceled { tstamp: now };

        let (cancel_result, denial_result) = tokio::join!(
            db.update_status_if_pending(quote.id, canceled),
            db.execute_governed_denial(denial),
        );
        assert_eq!(
            usize::from(cancel_result.is_ok()) + usize::from(denial_result.is_ok()),
            1
        );
        let stored = db.load(quote.id).await.unwrap().unwrap();
        match (cancel_result, denial_result) {
            (Ok(()), Err(crate::error::Error::CreditQuoteDenialConflict)) => {
                assert!(matches!(stored.status, quotes::Status::Canceled { .. }));
                assert!(stored.authorization_receipt().is_none());
            }
            (Err(crate::error::Error::QuotesRepository(_)), Ok(denial_receipt)) => {
                assert!(matches!(stored.status, quotes::Status::Denied { .. }));
                assert_eq!(stored.authorization_receipt(), Some(&denial_receipt));
            }
            unexpected => panic!("incoherent cancel/denial race result: {unexpected:?}"),
        }

        let fallback_offer =
            authorized_offer_candidate(&fallback, &format!("sha256:{}", "f".repeat(64)));
        db.execute_authorization(
            fallback_offer,
            ExposureReservationInput {
                exposure_limit_sat: 8_000_000,
                ..exposure(now)
            },
        )
        .await
        .expect("cancel and denial must not reserve exposure");
    }

    #[tokio::test]
    async fn governed_denial_durable_record_replays_from_another_surreal_handle() {
        let db = init_surreal_db().await;
        let other = db.independent_test_handle();
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-25T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let quote = pending_quote();
        db.store(quote.clone()).await.unwrap();
        let input = denial_input(
            &quote,
            &format!("sha256:{}", "c".repeat(64)),
            now,
            now + chrono::Duration::hours(1),
        );

        let committed = db.execute_governed_denial(input.clone()).await.unwrap();
        let replayed = other.execute_governed_denial(input).await.unwrap();
        assert_eq!(committed, replayed);
        let stored = db.load(quote.id).await.unwrap().unwrap();
        assert!(matches!(stored.status, quotes::Status::Denied { .. }));
        assert!(stored.authorization_receipt().is_some());
    }

    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn governed_denial_is_fail_closed_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBQuotes::from_pool(pool);
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-25T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let quote = pending_quote();
        db.store(quote.clone()).await.unwrap();

        assert!(matches!(
            db.execute_governed_denial(denial_input(
                &quote,
                &format!("sha256:{}", "c".repeat(64)),
                now,
                now + chrono::Duration::hours(1),
            ))
            .await,
            Err(crate::error::Error::CreditQuoteDenialUnavailable)
        ));
        let stored = db.load(quote.id).await.unwrap().unwrap();
        assert!(matches!(stored.status, quotes::Status::Pending { .. }));
        assert!(stored.authorization_receipt().is_none());
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

    fn denied_quote(now: TStamp) -> quotes::Quote {
        let mut quote = pending_quote();
        quote.bill.sum = bitcoin::Amount::from_sat(8_000_000);
        quote.bill.maturity_date = chrono::NaiveDate::from_ymd_opt(2027, 2, 6).unwrap();
        quote.submitted = now - chrono::Duration::minutes(1);
        quote.status = quotes::Status::Denied { tstamp: now };
        quote
    }

    fn reissued_quote(
        previous: &quotes::Quote,
        id: Uuid,
        wallet_pubkey: cashu::PublicKey,
        submitted: TStamp,
        ciphertext: &str,
        file_url: &str,
    ) -> quotes::Quote {
        let mut bill = previous.bill.clone();
        bill.shared_bill_data = ciphertext.to_owned();
        bill.file_urls = vec![url::Url::parse(file_url).unwrap()];
        quotes::Quote {
            status: quotes::Status::Pending { wallet_pubkey },
            id,
            bill,
            submitted,
            credit_program: previous.credit_program.clone(),
            authorization_receipt: None,
        }
    }

    #[tokio::test]
    async fn quote_reissue_is_atomic_and_semantically_idempotent() {
        quote_reissue_is_atomic_and_semantically_idempotent_for(init_inmemory_db()).await;
        quote_reissue_is_atomic_and_semantically_idempotent_for(init_surreal_db().await).await;
    }

    async fn quote_reissue_is_atomic_and_semantically_idempotent_for(db: impl Repository) {
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-10T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let previous = denied_quote(now);
        let reissued_id = Uuid::from_u128(0x300);
        let wallet = keys_test::publics()[0];
        let signed = crate::authorization::tests::signed_reissue_for(
            &previous,
            reissued_id,
            now - chrono::Duration::minutes(1),
            now + chrono::Duration::hours(1),
        );
        let quote = reissued_quote(
            &previous,
            reissued_id,
            wallet,
            now,
            "first-randomized-ciphertext",
            "https://files.invalid/first",
        );
        db.store(previous.clone()).await.unwrap();

        let (first, concurrent_retry) = tokio::join!(
            db.execute_quote_reissue(signed.clone(), quote.clone(), now),
            db.execute_quote_reissue(signed.clone(), quote.clone(), now),
        );
        assert_eq!(first.unwrap(), reissued_id);
        assert_eq!(concurrent_retry.unwrap(), reissued_id);

        // A lost response can be retried after both challenge expiry and quote progression. Core
        // re-encrypts bill/file payloads on every call, so replay identity is the validated bill,
        // exact wallet/program and permit authority rather than ciphertext, URL or timestamp.
        db.update_status_if_pending(
            reissued_id,
            quotes::Status::Denied {
                tstamp: now + chrono::Duration::minutes(2),
            },
        )
        .await
        .unwrap();
        let after_maturity = chrono::DateTime::parse_from_rfc3339("2027-02-07T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let retry = reissued_quote(
            &previous,
            reissued_id,
            wallet,
            after_maturity,
            "second-randomized-ciphertext",
            "https://files.invalid/second",
        );
        assert_eq!(
            db.execute_quote_reissue(signed, retry.clone(), after_maturity)
                .await
                .unwrap(),
            reissued_id
        );

        // Expiry renewal changes the permit bytes but preserves its immutable authority and qid.
        let successor = crate::authorization::tests::signed_reissue_for(
            &previous,
            reissued_id,
            after_maturity,
            after_maturity + chrono::Duration::hours(24),
        );
        assert_eq!(
            db.execute_quote_reissue(successor.clone(), retry.clone(), after_maturity)
                .await
                .unwrap(),
            reissued_id
        );

        let wrong_wallet = reissued_quote(
            &previous,
            reissued_id,
            keys_test::publics()[1],
            after_maturity,
            "third-randomized-ciphertext",
            "https://files.invalid/third",
        );
        assert!(matches!(
            db.execute_quote_reissue(successor, wrong_wallet, after_maturity)
                .await,
            Err(crate::error::Error::CreditQuoteReissueConflict)
        ));
    }

    #[tokio::test]
    async fn quote_reissue_transaction_converges_across_independent_service_handles() {
        let db = init_surreal_db().await;
        let other = db.independent_test_handle();
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-10T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let previous = denied_quote(now);
        let reissued_id = Uuid::from_u128(0x306);
        let signed = crate::authorization::tests::signed_reissue_for(
            &previous,
            reissued_id,
            now,
            now + chrono::Duration::hours(1),
        );
        let quote = reissued_quote(
            &previous,
            reissued_id,
            keys_test::publics()[0],
            now,
            "ciphertext",
            "https://files.invalid/concurrent",
        );
        db.store(previous).await.unwrap();

        let (left, right) = tokio::join!(
            db.execute_quote_reissue(signed.clone(), quote.clone(), now),
            other.execute_quote_reissue(signed, quote, now),
        );
        assert_eq!(left.unwrap(), reissued_id);
        assert_eq!(right.unwrap(), reissued_id);
        let stored = db.load(reissued_id).await.unwrap().unwrap();
        assert_eq!(
            db.search_by_bill(&stored.bill.id, &stored.bill.current_holder.node_id(),)
                .await
                .unwrap()
                .into_iter()
                .filter(|candidate| candidate.id == reissued_id)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn normal_enquiry_and_permitted_reissue_share_one_atomic_quote_head() {
        let db = init_surreal_db().await;
        let other = db.independent_test_handle();
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-10T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let previous = denied_quote(now);
        let normal_id = Uuid::from_u128(0x308);
        let reissued_id = Uuid::from_u128(0x309);
        let normal_wallet = keys_test::publics()[0];
        let reissue_wallet = keys_test::publics()[1];
        let normal = reissued_quote(
            &previous,
            normal_id,
            normal_wallet,
            now,
            "normal-ciphertext",
            "https://files.invalid/normal",
        );
        let reissued = reissued_quote(
            &previous,
            reissued_id,
            reissue_wallet,
            now,
            "reissue-ciphertext",
            "https://files.invalid/reissue",
        );
        let signed = crate::authorization::tests::signed_reissue_for(
            &previous,
            reissued_id,
            now,
            now + chrono::Duration::hours(1),
        );
        db.store(previous.clone()).await.unwrap();

        let (normal_result, reissue_result) = tokio::join!(
            db.store_if_latest(Some(previous.id), normal),
            other.execute_quote_reissue(signed, reissued, now),
        );
        match (normal_result, reissue_result) {
            (Ok(id), Err(crate::error::Error::CreditQuoteReissueConflict)) => {
                assert_eq!(id, normal_id);
            }
            (Err(crate::error::Error::CreditQuoteReissueConflict), Ok(id)) => {
                assert_eq!(id, reissued_id);
            }
            unexpected => panic!("normal/reissue race did not converge: {unexpected:?}"),
        }
        let current = db
            .search_by_bill(&previous.bill.id, &previous.bill.current_holder.node_id())
            .await
            .unwrap()
            .into_iter()
            .filter(|candidate| candidate.id != previous.id)
            .collect::<Vec<_>>();
        assert_eq!(current.len(), 1);
        assert!(current[0].id == normal_id || current[0].id == reissued_id);
    }

    #[tokio::test]
    async fn normal_enquiry_never_adopts_a_reissued_quote_for_another_wallet() {
        normal_enquiry_never_adopts_a_reissued_quote_for_another_wallet_in(init_inmemory_db())
            .await;
        normal_enquiry_never_adopts_a_reissued_quote_for_another_wallet_in(init_surreal_db().await)
            .await;
    }

    async fn normal_enquiry_never_adopts_a_reissued_quote_for_another_wallet_in(
        db: impl Repository,
    ) {
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-10T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let previous = denied_quote(now);
        let normal = reissued_quote(
            &previous,
            Uuid::from_u128(0x30a),
            keys_test::publics()[0],
            now,
            "normal-ciphertext",
            "https://files.invalid/normal",
        );
        let reissued_id = Uuid::from_u128(0x30b);
        let reissued = reissued_quote(
            &previous,
            reissued_id,
            keys_test::publics()[1],
            now,
            "reissue-ciphertext",
            "https://files.invalid/reissue",
        );
        let signed = crate::authorization::tests::signed_reissue_for(
            &previous,
            reissued_id,
            now,
            now + chrono::Duration::hours(1),
        );
        db.store(previous.clone()).await.unwrap();
        assert_eq!(
            db.execute_quote_reissue(signed, reissued, now)
                .await
                .unwrap(),
            reissued_id
        );
        assert!(matches!(
            db.store_if_latest(Some(previous.id), normal).await,
            Err(crate::error::Error::CreditQuoteReissueConflict)
        ));
    }

    #[tokio::test]
    async fn expired_unconsumed_quote_reissue_fails_closed() {
        expired_unconsumed_quote_reissue_fails_closed_for(init_inmemory_db()).await;
        expired_unconsumed_quote_reissue_fails_closed_for(init_surreal_db().await).await;
    }

    async fn expired_unconsumed_quote_reissue_fails_closed_for(db: impl Repository) {
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-10T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let previous = denied_quote(now);
        let reissued_id = Uuid::from_u128(0x301);
        let signed = crate::authorization::tests::signed_reissue_for(
            &previous,
            reissued_id,
            now - chrono::Duration::hours(2),
            now,
        );
        let quote = reissued_quote(
            &previous,
            reissued_id,
            keys_test::publics()[0],
            now,
            "ciphertext",
            "https://files.invalid/expired",
        );
        db.store(previous).await.unwrap();

        assert!(matches!(
            db.execute_quote_reissue(signed, quote, now).await,
            Err(crate::error::Error::CreditQuoteReissueInvalid)
        ));
        assert!(db.load(reissued_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn matured_unconsumed_quote_reissue_fails_closed() {
        matured_unconsumed_quote_reissue_fails_closed_for(init_inmemory_db()).await;
        matured_unconsumed_quote_reissue_fails_closed_for(init_surreal_db().await).await;
    }

    async fn matured_unconsumed_quote_reissue_fails_closed_for(db: impl Repository) {
        let denied_at = chrono::DateTime::parse_from_rfc3339("2027-02-05T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let consumed_at = chrono::DateTime::parse_from_rfc3339("2027-02-07T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let previous = denied_quote(denied_at);
        let reissued_id = Uuid::from_u128(0x305);
        let signed = crate::authorization::tests::signed_reissue_for(
            &previous,
            reissued_id,
            consumed_at,
            consumed_at + chrono::Duration::hours(1),
        );
        let quote = reissued_quote(
            &previous,
            reissued_id,
            keys_test::publics()[0],
            consumed_at,
            "ciphertext",
            "https://files.invalid/matured",
        );
        db.store(previous).await.unwrap();

        assert!(matches!(
            db.execute_quote_reissue(signed, quote, consumed_at).await,
            Err(crate::error::Error::InvalidInput(message)) if message.contains("maturity date")
        ));
        assert!(db.load(reissued_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn another_quote_at_the_denial_timestamp_blocks_reissue() {
        another_quote_at_the_denial_timestamp_blocks_reissue_for(init_inmemory_db()).await;
        another_quote_at_the_denial_timestamp_blocks_reissue_for(init_surreal_db().await).await;
    }

    async fn another_quote_at_the_denial_timestamp_blocks_reissue_for(db: impl Repository) {
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-10T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let previous = denied_quote(now);
        let reissued_id = Uuid::from_u128(0x302);
        let mut competing = reissued_quote(
            &previous,
            Uuid::from_u128(0x303),
            keys_test::publics()[0],
            previous.submitted,
            "competing",
            "https://files.invalid/competing",
        );
        competing.status = quotes::Status::Denied { tstamp: now };
        let signed = crate::authorization::tests::signed_reissue_for(
            &previous,
            reissued_id,
            now,
            now + chrono::Duration::hours(1),
        );
        let quote = reissued_quote(
            &previous,
            reissued_id,
            keys_test::publics()[0],
            now,
            "candidate",
            "https://files.invalid/candidate",
        );
        db.store(previous).await.unwrap();
        db.store(competing).await.unwrap();

        assert!(matches!(
            db.execute_quote_reissue(signed, quote, now).await,
            Err(crate::error::Error::CreditQuoteReissueConflict)
        ));
    }

    #[tokio::test]
    async fn quote_reissue_requires_the_exact_denied_source_and_preselected_target() {
        quote_reissue_requires_the_exact_denied_source_and_preselected_target_for(
            init_inmemory_db(),
        )
        .await;
        quote_reissue_requires_the_exact_denied_source_and_preselected_target_for(
            init_surreal_db().await,
        )
        .await;
    }

    async fn quote_reissue_requires_the_exact_denied_source_and_preselected_target_for(
        db: impl Repository,
    ) {
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-10T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let mut previous = denied_quote(now);
        previous.status = quotes::Status::Pending {
            wallet_pubkey: keys_test::publics()[0],
        };
        let reissued_id = Uuid::from_u128(0x307);
        let signed = crate::authorization::tests::signed_reissue_for(
            &previous,
            reissued_id,
            now,
            now + chrono::Duration::hours(1),
        );
        let quote = reissued_quote(
            &previous,
            reissued_id,
            keys_test::publics()[0],
            now,
            "candidate",
            "https://files.invalid/candidate",
        );
        db.store(previous.clone()).await.unwrap();
        assert!(matches!(
            db.execute_quote_reissue(signed.clone(), quote.clone(), now)
                .await,
            Err(crate::error::Error::CreditQuoteReissueConflict)
        ));

        db.update_status_if_pending(previous.id, quotes::Status::Denied { tstamp: now })
            .await
            .unwrap();
        let wrong_target = reissued_quote(
            &previous,
            Uuid::from_u128(0x308),
            keys_test::publics()[0],
            now,
            "candidate",
            "https://files.invalid/candidate",
        );
        assert!(matches!(
            db.execute_quote_reissue(signed, wrong_target, now).await,
            Err(crate::error::Error::CreditQuoteReissueConflict)
        ));
    }

    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn quote_reissue_fails_closed_on_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBQuotes::from_pool(pool);
        let now = TStamp::default();
        let previous = denied_quote(now);
        let reissued_id = Uuid::from_u128(0x304);
        let signed = crate::authorization::tests::signed_reissue_for(
            &previous,
            reissued_id,
            now,
            now + chrono::Duration::hours(1),
        );
        let quote = reissued_quote(
            &previous,
            reissued_id,
            keys_test::publics()[0],
            now,
            "candidate",
            "https://files.invalid/candidate",
        );
        db.store(previous).await.unwrap();
        assert!(matches!(
            db.execute_quote_reissue(signed, quote, now).await,
            Err(crate::error::Error::CreditQuoteReissueUnavailable)
        ));
    }
}
