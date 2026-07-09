// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    core::{BillId, NodeId},
    wire::{bill as wire_bill, quotes as wire_quotes},
};
use bitcoin as btc;
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence::Repository,
    quotes::{BillInfo, LightQuote, Quote, Status, StatusDiscriminants},
    TStamp,
};

// ----- end imports

// ---------- required traits
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ListFilters {
    pub bill_maturity_date_from: Option<chrono::NaiveDate>,
    pub bill_maturity_date_to: Option<chrono::NaiveDate>,
    pub status: Option<StatusDiscriminants>,
    pub bill_id: Option<BillId>,
    pub bill_drawee_id: Option<NodeId>,
    pub bill_drawer_id: Option<NodeId>,
    pub bill_payer_id: Option<NodeId>,
    pub bill_holder_id: Option<NodeId>,
}

#[derive(Debug, Clone)]
pub enum SortOrder {
    BillMaturityDateAsc,
    BillMaturityDateDesc,
}

pub enum MintingStatus {
    Disabled,
    Enabled(cashu::Amount),
}
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait WdcClient: Send + Sync {
    async fn get_keyset_with_expiration_date(
        &self,
        expiration_date: chrono::NaiveDate,
    ) -> Result<cashu::Id>;
    async fn get_keys(&self, keyset_id: cashu::Id) -> Result<cashu::KeySet>;
    async fn add_new_mint_operation(
        &self,
        qid: Uuid,
        kid: cashu::Id,
        pk: cashu::PublicKey,
        target: cashu::Amount,
        bill_id: BillId,
    ) -> Result<()>;
    async fn sign(&self, msgs: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>>;
    async fn get_minting_status(&self, qid: Uuid) -> Result<MintingStatus>;
    async fn validate_and_decrypt_shared_bill(
        &self,
        shared_bill: &wire_quotes::SharedBill,
    ) -> Result<wire_quotes::BillInfo>;
    async fn validate_endorsed_bill_matches_shared_bill(
        &self,
        bill_id: BillId,
        shared_bill_data: String,
    ) -> Result<bool>;
    async fn get_shared_ebill_history(
        &self,
        bill_id: BillId,
        shared_bill_data: String,
    ) -> Result<Vec<wire_bill::BillHistoryBlock>>;
    async fn get_ebill(&self, bid: BillId) -> Result<wire_bill::BitcreditBill>;
    async fn collect_fees(&self, proofs: Vec<cashu::Proof>) -> Result<()>;
}

// ---------- Service
pub struct Service {
    pub wdc_client: Box<dyn WdcClient + Send + Sync>,
    pub quotes: Box<dyn Repository + Send + Sync>,
    pub mint_url: cashu::MintUrl,
}

impl Service {
    pub(crate) const USER_DECISION_RETENTION: chrono::Duration = chrono::Duration::days(1);

    async fn _lookup(&self, qid: uuid::Uuid, now: TStamp) -> Result<Quote> {
        let mut quote = self
            .quotes
            .load(qid)
            .await?
            .ok_or(Error::ResourceNotFound(qid.to_string()))?;
        let changed = quote.check_expire(now);
        if changed {
            self.quotes
                .update_status_if_offered(quote.id, quote.status.clone())
                .await?;
        }
        Ok(quote)
    }

    async fn new_quote(
        &self,
        bill: BillInfo,
        minting_pub_key: cashu::PublicKey,
        submitted: TStamp,
    ) -> Result<Uuid> {
        let quote = Quote::new(bill, minting_pub_key, submitted);
        let qid = quote.id;
        self.quotes.store(quote).await?;
        Ok(qid)
    }

    pub async fn validate_and_decrypt_shared_bill(
        &self,
        shared_bill: &wire_quotes::SharedBill,
    ) -> Result<wire_quotes::BillInfo> {
        self.wdc_client
            .validate_and_decrypt_shared_bill(shared_bill)
            .await
    }

    pub async fn enquire(
        &self,
        bill: BillInfo,
        pub_key: cashu::PublicKey,
        submitted: TStamp,
    ) -> Result<uuid::Uuid> {
        validate_basic_ebill_rules(&bill)?;
        let holder_id = &bill.endorsees.last().unwrap_or(&bill.payee).node_id();
        let mut quotes = self.quotes.search_by_bill(&bill.id, holder_id).await?;

        // pick the more recent quote for this eBill/endorser
        quotes.sort_by_key(|q| q.submitted);
        if let Some(last) = quotes.last_mut() {
            let changed = last.check_expire(submitted);
            if changed {
                self.quotes
                    .update_status_if_offered(last.id, last.status.clone())
                    .await?;
            }
        }
        match quotes.last() {
            Some(Quote {
                id,
                status: Status::Canceled { tstamp },
                ..
            })
            | Some(Quote {
                id,
                status: Status::Denied { tstamp },
                ..
            })
            | Some(Quote {
                id,
                status: Status::OfferExpired { tstamp, .. },
                ..
            })
            | Some(Quote {
                id,
                status: Status::Rejected { tstamp, .. },
                ..
            }) => {
                if (submitted - tstamp) > Self::USER_DECISION_RETENTION {
                    self.new_quote(bill, pub_key, submitted).await
                } else {
                    Ok(*id)
                }
            }
            Some(Quote {
                id,
                status: Status::Pending { .. },
                ..
            })
            | Some(Quote {
                id,
                status: Status::Offered { .. },
                ..
            })
            | Some(Quote {
                id,
                status: Status::Accepted { .. },
                ..
            })
            | Some(Quote {
                id,
                status: Status::FailedEbillValidation { .. },
                ..
            }) => Ok(*id),
            Some(Quote {
                id,
                status: Status::MintingEnabled { .. },
                ..
            }) => Ok(*id),
            None => self.new_quote(bill, pub_key, submitted).await,
        }
    }

    pub async fn cancel(&self, id: uuid::Uuid, submitted: TStamp) -> Result<()> {
        let old = self.quotes.load(id).await?;
        if old.is_none() {
            return Err(Error::ResourceNotFound(id.to_string()));
        }
        let mut quote = old.unwrap();
        quote.cancel(submitted)?;
        self.quotes
            .update_status_if_pending(quote.id, quote.status)
            .await?;
        Ok(())
    }

    pub async fn deny(&self, id: uuid::Uuid, submitted: TStamp) -> Result<()> {
        let old = self.quotes.load(id).await?;
        if old.is_none() {
            return Err(Error::ResourceNotFound(id.to_string()));
        }
        let mut quote = old.unwrap();
        quote.deny(submitted)?;
        self.quotes
            .update_status_if_pending(quote.id, quote.status)
            .await?;
        Ok(())
    }

    pub async fn reject(&self, id: uuid::Uuid, tstamp: TStamp) -> Result<()> {
        let old = self.quotes.load(id).await?;
        if old.is_none() {
            return Err(Error::ResourceNotFound(id.to_string()));
        }
        let mut quote = old.unwrap();
        quote.reject(tstamp)?;
        self.quotes
            .update_status_if_offered(quote.id, quote.status)
            .await?;
        Ok(())
    }

    pub async fn accept(&self, id: uuid::Uuid, submitted: TStamp) -> Result<()> {
        let old = self.quotes.load(id).await?;
        if old.is_none() {
            return Err(Error::ResourceNotFound(id.to_string()));
        }
        let mut quote = old.unwrap();
        quote.accept(submitted)?;
        self.quotes
            .update_status_if_offered(quote.id, quote.status)
            .await?;
        Ok(())
    }

    pub async fn lookup(&self, qid: uuid::Uuid, now: TStamp) -> Result<Quote> {
        let quote = self._lookup(qid, now).await?;
        Ok(quote)
    }

    pub async fn list_pendings(&self, since: Option<TStamp>) -> Result<Vec<uuid::Uuid>> {
        self.quotes.list_pendings(since).await
    }

    pub async fn list_light(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
        now: TStamp,
    ) -> Result<Vec<LightQuote>> {
        let mut lights = self.quotes.list_light(filters, sort).await?;

        for light in lights.iter_mut() {
            if matches!(light.status, StatusDiscriminants::Offered) {
                let mut quote = self
                    .quotes
                    .load(light.id)
                    .await?
                    .ok_or(Error::InternalServer(String::from(
                        "light quote ID not found in quote",
                    )))?;
                let changed = quote.check_expire(now);
                if changed {
                    self.quotes
                        .update_status_if_offered(light.id, quote.status.clone())
                        .await?;
                    light.status = StatusDiscriminants::from(quote.status.clone());
                }
            }
        }
        Ok(lights)
    }

    pub async fn offer(
        &self,
        qid: uuid::Uuid,
        discounted: btc::Amount,
        submitted: TStamp,
        ttl: Option<TStamp>,
    ) -> Result<(btc::Amount, TStamp)> {
        let mut quote = self._lookup(qid, submitted).await?;
        let Status::Pending { .. } = quote.status else {
            return Err(Error::InvalidQuoteStatus(
                qid,
                StatusDiscriminants::Pending,
                StatusDiscriminants::from(quote.status.clone()),
            ));
        };
        let expiration_date = calculate_expiration_from_maturity(quote.bill.maturity_date);
        let kid = self
            .wdc_client
            .get_keyset_with_expiration_date(expiration_date)
            .await?;
        let expiration = ttl.unwrap_or(calculate_default_expiration_date_for_quote(submitted));
        quote.offer(kid, expiration, discounted)?;
        self.quotes
            .update_status_if_pending(quote.id, quote.status)
            .await?;
        Ok((discounted, expiration))
    }

    pub async fn set_failed_ebill_validation(&self, qid: uuid::Uuid) -> Result<()> {
        let mut quote = self
            .quotes
            .load(qid)
            .await?
            .ok_or(Error::ResourceNotFound(qid.to_string()))?;
        let Status::Accepted { .. } = quote.status else {
            return Err(Error::InvalidQuoteStatus(
                qid,
                StatusDiscriminants::Accepted,
                StatusDiscriminants::from(quote.status.clone()),
            ));
        };
        quote.set_failed_ebill_validation()?;
        self.quotes
            .update_status_if_accepted(quote.id, quote.status)
            .await?;
        Ok(())
    }

    pub async fn enable_minting_manual_override(&self, qid: uuid::Uuid) -> Result<()> {
        let mut quote = self
            .quotes
            .load(qid)
            .await?
            .ok_or(Error::ResourceNotFound(qid.to_string()))?;
        let Status::FailedEbillValidation {
            keyset_id,
            discounted,
            wallet_pubkey,
        } = quote.status
        else {
            return Err(Error::InvalidQuoteStatus(
                qid,
                StatusDiscriminants::FailedEbillValidation,
                StatusDiscriminants::from(quote.status.clone()),
            ));
        };
        let fees_amount = quote.bill.sum - discounted;
        let fees_amount = cashu::Amount::from(fees_amount.to_sat());
        quote.override_failed_ebill_validation(fees_amount)?;
        self.trigger_enable_minting_operations(
            qid,
            keyset_id,
            wallet_pubkey,
            fees_amount,
            discounted,
            quote.bill.id.clone(),
        )
        .await?;
        self.quotes
            .update_status_if_failedebillvalidation(quote.id, quote.status)
            .await?;
        Ok(())
    }

    pub async fn enable_minting(&self, qid: uuid::Uuid) -> Result<()> {
        let mut quote = self
            .quotes
            .load(qid)
            .await?
            .ok_or(Error::ResourceNotFound(qid.to_string()))?;
        let Status::Accepted {
            keyset_id,
            discounted,
            wallet_pubkey,
        } = quote.status
        else {
            return Err(Error::InvalidQuoteStatus(
                qid,
                StatusDiscriminants::Accepted,
                StatusDiscriminants::from(quote.status.clone()),
            ));
        };
        let fees_amount = quote.bill.sum - discounted;
        let fees_amount = cashu::Amount::from(fees_amount.to_sat());
        quote.start_minting(fees_amount)?;
        self.trigger_enable_minting_operations(
            qid,
            keyset_id,
            wallet_pubkey,
            fees_amount,
            discounted,
            quote.bill.id.clone(),
        )
        .await?;
        self.quotes
            .update_status_if_accepted(quote.id, quote.status)
            .await?;
        Ok(())
    }

    async fn trigger_enable_minting_operations(
        &self,
        qid: uuid::Uuid,
        keyset_id: cashu::Id,
        wallet_pubkey: cashu::PublicKey,
        fees_amount: cashu::Amount,
        discounted: btc::Amount,
        bill_id: BillId,
    ) -> Result<()> {
        let keys = self.wdc_client.get_keys(keyset_id).await?;
        let fees = mint_fees(self.wdc_client.as_ref(), fees_amount, keys).await?;
        let discounted_amount = cashu::Amount::from(discounted.to_sat());
        self.wdc_client
            .add_new_mint_operation(qid, keyset_id, wallet_pubkey, discounted_amount, bill_id)
            .await?;
        self.wdc_client.collect_fees(fees).await?;
        Ok(())
    }

    pub async fn check_if_endorsed_bill_is_valid(
        &self,
        bill_id: BillId,
        quote: Quote,
    ) -> Result<bool> {
        let res = self
            .wdc_client
            .validate_endorsed_bill_matches_shared_bill(bill_id, quote.bill.shared_bill_data)
            .await?;
        Ok(res)
    }

    pub async fn get_shared_ebill_history(
        &self,
        qid: uuid::Uuid,
    ) -> Result<Vec<wire_bill::BillHistoryBlock>> {
        let quote = self
            .quotes
            .load(qid)
            .await?
            .ok_or(Error::ResourceNotFound(qid.to_string()))?;
        let history_blocks = self
            .wdc_client
            .get_shared_ebill_history(quote.bill.id, quote.bill.shared_bill_data)
            .await?;
        Ok(history_blocks)
    }
}

pub fn calculate_default_expiration_date_for_quote(now: crate::TStamp) -> super::TStamp {
    now + chrono::Duration::days(2)
}

pub fn calculate_expiration_from_maturity(maturity_date: chrono::NaiveDate) -> chrono::NaiveDate {
    maturity_date + chrono::Duration::days(2)
}

async fn mint_fees(
    keyscl: &dyn WdcClient,
    fees_amount: cashu::Amount,
    keys: cashu::KeySet,
) -> Result<Vec<cashu::Proof>> {
    let premint = cashu::PreMintSecrets::random(
        keys.id,
        fees_amount,
        &cashu::amount::SplitTarget::None,
        &bcr_wdc_utils::keys::to_fee_and_amounts(&keys),
    )
    .map_err(|e| Error::InternalServer(format!("mint_fees(): PreMintSecrets::random(): {e}")))?;
    let signatures = keyscl.sign(&premint.blinded_messages()).await?;
    let (rs, secrets) = premint
        .secrets
        .into_iter()
        .map(|secret| (secret.r, secret.secret))
        .unzip();
    let prfs = cashu::dhke::construct_proofs(signatures, rs, secrets, &keys.keys)
        .map_err(|e| Error::InternalServer(format!("mint_fees(): construct_proofs(): {e}")))?;
    Ok(prfs)
}

fn validate_basic_ebill_rules(bill: &BillInfo) -> Result<()> {
    if bill.maturity_date <= chrono::Utc::now().date_naive() {
        return Err(Error::InvalidInput(String::from("maturity date > today")));
    }
    if bill.sum <= btc::Amount::ONE_SAT || bill.sum > bitcoin::Amount::MAX_MONEY {
        return Err(Error::InvalidInput(format!(
            "{} < bill_amount < {}",
            btc::Amount::ONE_SAT,
            btc::Amount::MAX_MONEY
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::MockRepository;
    use bcr_common::{core_tests, wire_tests};
    use bcr_ebill_core::protocol::blockchain::bill::participant::BillParticipant;
    use bcr_wdc_utils::{convert, keys::test_utils as keys_utils};
    use mockall::predicate::*;
    use rand::Rng;
    use std::str::FromStr;

    pub const TEST_URL: &str = "http://localhost:8000";

    fn generate_random_bill() -> BillInfo {
        let mut rng = rand::thread_rng();
        let holder =
            convert::billidentparticipant_wire2ebill(wire_tests::random_identity_public_data().1)
                .unwrap();
        BillInfo {
            id: core_tests::random_bill_id(),
            drawee: convert::billidentparticipant_wire2ebill(
                wire_tests::random_identity_public_data().1,
            )
            .unwrap(),
            drawer: convert::billidentparticipant_wire2ebill(
                wire_tests::random_identity_public_data().1,
            )
            .unwrap(),
            payee: BillParticipant::Ident(holder.clone()),
            current_holder: BillParticipant::Ident(holder),
            endorsees: Default::default(),
            sum: btc::Amount::from_sat(rng.gen_range(1000..100000)),
            maturity_date: (chrono::Utc::now() + chrono::Duration::days(rng.gen_range(10..30)))
                .date_naive(),
            file_urls: Vec::default(),
            shared_bill_data: String::default(),
        }
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_not_present() {
        let mut quotes = MockRepository::new();
        quotes.expect_search_by_bill().returning(|_, _| Ok(vec![]));
        quotes.expect_store().returning(|_| Ok(()));
        let wdc_client = MockWdcClient::new();

        let rnd_bill = generate_random_bill();
        let service = Service {
            quotes: Box::new(quotes),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
        };
        let test = service
            .enquire(rnd_bill, keys_utils::publics()[0], chrono::Utc::now())
            .await;
        assert!(test.is_ok());
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_pending() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let wallet_pubkey = keys_utils::publics()[0];
        let mut repo = MockRepository::new();
        let cloned = rnd_bill.clone();
        repo.expect_search_by_bill()
            .with(
                eq(rnd_bill.id.clone()),
                eq(rnd_bill.payee.node_id().clone()),
            )
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: Status::Pending { wallet_pubkey },
                    id,
                    bill: cloned.clone(),
                    submitted: chrono::Utc::now(),
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let wdc_client = MockWdcClient::new();

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
        };
        let test_id = service
            .enquire(rnd_bill, wallet_pubkey, chrono::Utc::now())
            .await;
        assert!(test_id.is_ok());
        assert_eq!(id, test_id.unwrap());
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_denied() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let public_key = keys_utils::publics()[0];
        let cloned = rnd_bill.clone();
        let now = TStamp::from_timestamp(10000, 0).unwrap();
        let mut repo = MockRepository::new();
        repo.expect_search_by_bill()
            .with(
                eq(rnd_bill.id.clone()),
                eq(rnd_bill.payee.node_id().clone()),
            )
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: Status::Denied { tstamp: now },
                    id,
                    bill: cloned.clone(),
                    submitted: now,
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let wdc_client = MockWdcClient::new();

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
        };
        let test_id = service.enquire(rnd_bill, public_key, now).await.unwrap();
        assert_eq!(id, test_id);
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_offered() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let keyset_id = core_tests::generate_random_ecash_keyset().0.id;
        let wallet_pubkey = keys_utils::publics()[0];
        let now = TStamp::from_timestamp(10000, 0).unwrap();
        let cloned = rnd_bill.clone();
        let mut repo = MockRepository::new();
        repo.expect_search_by_bill()
            .with(
                eq(rnd_bill.id.clone()),
                eq(rnd_bill.payee.node_id().clone()),
            )
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: Status::Offered {
                        keyset_id,
                        ttl: now + chrono::Duration::days(1),
                        discounted: rnd_bill.sum,
                        wallet_pubkey,
                    },
                    id,
                    bill: cloned.clone(),
                    submitted: now,
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let wdc_client = MockWdcClient::new();

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
        };
        let test_id = service.enquire(rnd_bill, wallet_pubkey, now).await.unwrap();
        assert_eq!(id, test_id);
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_offered_but_expired() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let cloned = rnd_bill.clone();
        let keyset_id = core_tests::generate_random_ecash_keyset().0.id;
        let wallet_pubkey = keys_utils::publics()[0];
        let mut repo = MockRepository::new();
        let now = TStamp::from_timestamp(10000, 0).unwrap();
        repo.expect_search_by_bill()
            .with(
                eq(rnd_bill.id.clone()),
                eq(rnd_bill.payee.node_id().clone()),
            )
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: Status::Offered {
                        keyset_id,
                        ttl: now,
                        discounted: rnd_bill.sum,
                        wallet_pubkey,
                    },
                    id,
                    bill: cloned.clone(),
                    submitted: now,
                }])
            });
        repo.expect_update_status_if_offered()
            .returning(|_, _| Ok(()));
        let wdc_client = MockWdcClient::new();

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
        };
        let test_id = service
            .enquire(rnd_bill, wallet_pubkey, now + chrono::Duration::seconds(1))
            .await
            .unwrap();
        assert_eq!(id, test_id);
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_offered_expired_retention_passed() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let cloned = rnd_bill.clone();
        let keyset_id = core_tests::generate_random_ecash_keyset().0.id;
        let wallet_pubkey = keys_utils::publics()[0];
        let mut repo = MockRepository::new();
        let now = TStamp::from_timestamp(10000, 0).unwrap();
        repo.expect_search_by_bill()
            .with(
                eq(rnd_bill.id.clone()),
                eq(rnd_bill.payee.node_id().clone()),
            )
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: Status::Offered {
                        keyset_id,
                        ttl: now,
                        discounted: rnd_bill.sum,
                        wallet_pubkey,
                    },
                    id,
                    bill: cloned.clone(),
                    submitted: now,
                }])
            });
        repo.expect_update_status_if_offered()
            .returning(|_, _| Ok(()));
        repo.expect_store().returning(|_| Ok(()));
        let wdc_client = MockWdcClient::new();

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
        };
        let submitted = now + Service::USER_DECISION_RETENTION + chrono::Duration::seconds(1);
        let test_id = service.enquire(rnd_bill, wallet_pubkey, submitted).await;
        assert!(test_id.is_ok());
        assert_ne!(id, test_id.unwrap());
    }

    #[tokio::test]
    async fn test_enable_minting_manual_override_quote_not_found() {
        let qid = Uuid::new_v4();
        let mut repo = MockRepository::new();
        repo.expect_load()
            .with(eq(qid))
            .times(1)
            .returning(|_| Ok(None));
        let wdc_client = MockWdcClient::new();

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
        };
        let res = service.enable_minting_manual_override(qid).await;
        assert!(matches!(
            res,
            Err(Error::ResourceNotFound(id)) if id == qid.to_string()
        ));
    }

    #[tokio::test]
    async fn test_enable_minting_manual_override_invalid_status() {
        let qid = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let wallet_pubkey = keys_utils::publics()[0];
        let mut repo = MockRepository::new();
        repo.expect_load()
            .with(eq(qid))
            .times(1)
            .returning(move |_| {
                Ok(Some(Quote {
                    id: qid,
                    status: Status::Pending { wallet_pubkey },
                    bill: rnd_bill.clone(),
                    submitted: chrono::Utc::now(),
                }))
            });

        let wdc_client = MockWdcClient::new();

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
        };
        let res = service.enable_minting_manual_override(qid).await;
        assert!(matches!(
            res,
            Err(Error::InvalidQuoteStatus(
                id,
                StatusDiscriminants::FailedEbillValidation,
                StatusDiscriminants::Pending,
            )) if id == qid
        ));
    }

    #[tokio::test]
    async fn test_enable_minting_manual_override_success() {
        let qid = Uuid::new_v4();
        let mut quote = Quote::new(
            generate_random_bill(),
            keys_utils::publics()[0],
            chrono::Utc::now(),
        );
        quote.id = qid;
        let (_keyset_info, signing_keyset) = core_tests::generate_random_ecash_keyset();
        let keyset = cashu::KeySet {
            id: signing_keyset.id,
            unit: signing_keyset.unit.clone(),
            active: None,
            keys: signing_keyset.keys.clone().into(),
            input_fee_ppk: signing_keyset.input_fee_ppk,
            final_expiry: signing_keyset.final_expiry,
        };
        let keyset_id = keyset.id;
        let wallet_pubkey = keys_utils::publics()[0];
        let fee = cashu::Amount::from(10);
        let discounted = quote.bill.sum - btc::Amount::from_sat(10);
        let bill_id = quote.bill.id.clone();
        quote.status = Status::FailedEbillValidation {
            keyset_id,
            discounted,
            wallet_pubkey,
        };

        let mut repo = MockRepository::new();
        repo.expect_load()
            .with(eq(qid))
            .times(1)
            .returning(move |_| {
                Ok(Some(Quote {
                    id: qid,
                    status: Status::FailedEbillValidation {
                        keyset_id,
                        discounted,
                        wallet_pubkey,
                    },
                    bill: quote.bill.clone(),
                    submitted: chrono::Utc::now(),
                }))
            });
        repo.expect_update_status_if_failedebillvalidation()
            .withf(move |id, status| {
                *id == qid
                    && matches!(
                        status,
                        Status::MintingEnabled {
                            keyset_id: actual_keyset_id,
                            discounted: actual_discounted,
                            wallet_pubkey: actual_wallet_pubkey,
                            fee: actual_fee
                        } if *actual_keyset_id == keyset_id
                            && *actual_discounted == discounted
                            && *actual_wallet_pubkey == wallet_pubkey
                            && *actual_fee == fee
                    )
            })
            .times(1)
            .returning(|_, _| Ok(()));

        let mut wdc_client = MockWdcClient::new();
        wdc_client
            .expect_get_keys()
            .with(eq(keyset_id))
            .times(1)
            .returning(move |_| Ok(keyset.clone()));

        let signing_keyset = signing_keyset.clone();
        wdc_client.expect_sign().times(1).returning(move |msgs| {
            let amounts = msgs.iter().map(|msg| msg.amount).collect::<Vec<_>>();
            Ok(core_tests::generate_ecash_signatures(
                &signing_keyset,
                &amounts,
            ))
        });
        wdc_client
            .expect_add_new_mint_operation()
            .withf(
                move |actual_qid,
                      actual_keyset_id,
                      actual_wallet_pubkey,
                      target,
                      actual_bill_id| {
                    *actual_qid == qid
                        && *actual_keyset_id == keyset_id
                        && *actual_wallet_pubkey == wallet_pubkey
                        && *target == cashu::Amount::from(discounted.to_sat())
                        && *actual_bill_id == bill_id
                },
            )
            .times(1)
            .returning(|_, _, _, _, _| Ok(()));
        wdc_client
            .expect_collect_fees()
            .times(1)
            .returning(|_| Ok(()));

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
        };
        let res = service.enable_minting_manual_override(qid).await;
        assert!(res.is_ok());
    }
}
