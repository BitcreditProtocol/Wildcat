// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    core::{BillId, NodeId},
    wallet::Token,
    wire::quotes as wire_quotes,
};
use bitcoin::Amount;
use futures::future::JoinAll;
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
pub trait KeysHandler: Send + Sync {
    async fn get_keyset_with_redemption_date(
        &self,
        redemption_date: chrono::NaiveDate,
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
    async fn sign(&self, msg: &cashu::BlindedMessage) -> Result<cashu::BlindSignature>;
    async fn get_minting_status(&self, qid: Uuid) -> Result<MintingStatus>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait EBillNode: Sync {
    async fn validate_and_decrypt_shared_bill(
        &self,
        shared_bill: &wire_quotes::SharedBill,
    ) -> Result<wire_quotes::BillInfo>;
}

// ---------- Service
#[derive(Clone)]
pub struct Service {
    pub keys_hndlr: Arc<dyn KeysHandler + Send + Sync>,
    pub quotes: Arc<dyn Repository + Send + Sync>,
    pub ebill: Arc<dyn EBillNode + Send + Sync>,
    pub mint_url: cashu::MintUrl,
}

impl Service {
    pub(crate) const USER_DECISION_RETENTION: chrono::Duration = chrono::Duration::days(1);

    async fn _lookup(&self, qid: uuid::Uuid, now: TStamp) -> Result<Quote> {
        let mut quote = self
            .quotes
            .load(qid)
            .await?
            .ok_or(Error::QuoteIDNotFound(qid))?;
        let changed = quote.check_expire(now);
        if changed {
            self.quotes
                .update_status_if_offered(quote.id, quote.status.clone())
                .await?;
        }
        Ok(quote)
    }

    pub async fn new_quote(
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
        self.ebill
            .validate_and_decrypt_shared_bill(shared_bill)
            .await
    }

    pub async fn enquire(
        &self,
        bill: BillInfo,
        pub_key: cashu::PublicKey,
        submitted: TStamp,
    ) -> Result<uuid::Uuid> {
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
            return Err(Error::QuoteIDNotFound(id));
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
            return Err(Error::QuoteIDNotFound(id));
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
            return Err(Error::QuoteIDNotFound(id));
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
            return Err(Error::QuoteIDNotFound(id));
        }
        let mut quote = old.unwrap();
        quote.accept(submitted)?;
        self.quotes
            .update_status_if_offered(quote.id, quote.status)
            .await?;
        Ok(())
    }

    pub async fn lookup(&self, qid: uuid::Uuid, now: TStamp) -> Result<(Quote, MintingStatus)> {
        let quote = self._lookup(qid, now).await?;
        let minting_status = if matches!(quote.status, Status::MintingEnabled { .. }) {
            self.keys_hndlr.get_minting_status(qid).await?
        } else {
            MintingStatus::Disabled
        };
        Ok((quote, minting_status))
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
        discounted: Amount,
        submitted: TStamp,
        ttl: Option<TStamp>,
    ) -> Result<(Amount, TStamp)> {
        let mut quote = self._lookup(qid, submitted).await?;
        let Status::Pending { .. } = quote.status else {
            return Err(Error::InvalidQuoteStatus(
                qid,
                StatusDiscriminants::Pending,
                StatusDiscriminants::from(quote.status.clone()),
            ));
        };
        let maturity_date = quote.bill.maturity_date;
        let kid = self
            .keys_hndlr
            .get_keyset_with_redemption_date(maturity_date)
            .await?;
        let expiration = ttl.unwrap_or(calculate_default_expiration_date_for_quote(submitted));
        quote.offer(kid, expiration, discounted)?;
        self.quotes
            .update_status_if_pending(quote.id, quote.status)
            .await?;
        Ok((discounted, expiration))
    }

    pub async fn enable_minting(&self, qid: uuid::Uuid) -> Result<()> {
        let mut quote = self
            .quotes
            .load(qid)
            .await?
            .ok_or(Error::QuoteIDNotFound(qid))?;
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
        let keys = self.keys_hndlr.get_keys(keyset_id).await?;
        let fees_token = mint_fees(
            self.keys_hndlr.as_ref(),
            fees_amount,
            keys,
            self.mint_url.clone(),
            &quote.bill.id,
        )
        .await?;
        let discounted_amount = cashu::Amount::from(discounted.to_sat());
        quote.start_minting(fees_token)?;
        self.quotes
            .update_status_if_accepted(quote.id, quote.status)
            .await?;
        self.keys_hndlr
            .add_new_mint_operation(
                qid,
                keyset_id,
                wallet_pubkey,
                discounted_amount,
                quote.bill.id.clone(),
            )
            .await?;
        Ok(())
    }
}

pub fn calculate_default_expiration_date_for_quote(now: crate::TStamp) -> super::TStamp {
    now + chrono::Duration::days(2)
}

async fn mint_fees(
    keyscl: &dyn KeysHandler,
    fees_amount: cashu::Amount,
    keys: cashu::KeySet,
    mint_url: cashu::MintUrl,
    billid: &BillId,
) -> Result<Token> {
    let premints =
        cashu::PreMintSecrets::random(keys.id, fees_amount, &cashu::amount::SplitTarget::None)
            .map_err(|e| {
                Error::InternalServer(format!("mint_fees(): PreMintSecrets::random(): {e}"))
            })?;
    let blinds = premints.blinded_messages();
    let joined: JoinAll<_> = blinds.iter().map(|blind| keyscl.sign(blind)).collect();
    let signatures: Vec<cashu::BlindSignature> = joined.await.into_iter().collect::<Result<_>>()?;
    let mut proofs = Vec::new();
    for (signature, premint) in signatures.into_iter().zip(premints.iter()) {
        let proof =
            bcr_common::core::signature::unblind_ecash_signature(&keys, premint.clone(), signature)
                .map_err(|e| Error::InternalServer(e.to_string()))?;
        proofs.push(proof);
    }
    let token = Token::new_bitcr(mint_url, proofs, Some(billid.to_string()), keys.unit);
    Ok(token)
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
            sum: Amount::from_sat(rng.gen_range(1000..100000)),
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
        let keys_hndlr = MockKeysHandler::new();
        let ebill = MockEBillNode::new();

        let rnd_bill = generate_random_bill();
        let service = Service {
            quotes: Arc::new(quotes),
            keys_hndlr: Arc::new(keys_hndlr),
            ebill: Arc::new(ebill),
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
        let keys_hndlr = MockKeysHandler::new();
        let ebill = MockEBillNode::new();

        let service = Service {
            quotes: Arc::new(repo),
            keys_hndlr: Arc::new(keys_hndlr),
            ebill: Arc::new(ebill),
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
        let keys_hndlr = MockKeysHandler::new();
        let ebill = MockEBillNode::new();

        let service = Service {
            quotes: Arc::new(repo),
            keys_hndlr: Arc::new(keys_hndlr),
            ebill: Arc::new(ebill),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
        };
        let test_id = service.enquire(rnd_bill, public_key, now).await.unwrap();
        assert_eq!(id, test_id);
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_offered() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let keyset_id = keys_utils::generate_random_keysetid();
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
        let keys_hndlr = MockKeysHandler::new();
        let ebill = MockEBillNode::new();

        let service = Service {
            quotes: Arc::new(repo),
            keys_hndlr: Arc::new(keys_hndlr),
            ebill: Arc::new(ebill),
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
        let keyset_id = keys_utils::generate_random_keysetid();
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
        let keys_hndlr = MockKeysHandler::new();
        let ebill = MockEBillNode::new();

        let service = Service {
            quotes: Arc::new(repo),
            keys_hndlr: Arc::new(keys_hndlr),
            ebill: Arc::new(ebill),
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
        let keyset_id = keys_utils::generate_random_keysetid();
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
        let keys_hndlr = MockKeysHandler::new();
        let ebill = MockEBillNode::new();

        let service = Service {
            quotes: Arc::new(repo),
            keys_hndlr: Arc::new(keys_hndlr),
            ebill: Arc::new(ebill),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
        };
        let submitted = now + Service::USER_DECISION_RETENTION + chrono::Duration::seconds(1);
        let test_id = service.enquire(rnd_bill, wallet_pubkey, submitted).await;
        assert!(test_id.is_ok());
        assert_ne!(id, test_id.unwrap());
    }
}
