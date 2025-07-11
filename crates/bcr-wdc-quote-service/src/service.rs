// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_ebill_core::{bill::BillId, NodeId};
use bcr_wdc_webapi::quotes::SharedBill;
use bitcoin::Amount;
use cashu::{nut00 as cdk00, nut01 as cdk01, nut02 as cdk02};
use futures::future::JoinAll;
use uuid::Uuid;
// ----- local imports
use crate::error::{Error, Result};
use crate::quotes::{BillInfo, LightQuote, Quote, Status, StatusDiscriminants};
use crate::TStamp;

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

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository {
    async fn load(&self, id: uuid::Uuid) -> Result<Option<Quote>>;
    async fn update_status_if_pending(&self, id: uuid::Uuid, quote: Status) -> Result<()>;
    async fn update_status_if_offered(&self, id: uuid::Uuid, quote: Status) -> Result<()>;
    async fn list_pendings(&self, since: Option<TStamp>) -> Result<Vec<Uuid>>;
    async fn list_light(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
    ) -> Result<Vec<LightQuote>>;
    async fn search_by_bill(&self, bill: &BillId, endorser: &NodeId) -> Result<Vec<Quote>>;
    async fn store(&self, quote: Quote) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeysHandler {
    async fn generate(
        &self,
        qid: Uuid,
        amount: Amount,
        pk: cdk01::PublicKey,
        maturity_date: TStamp,
    ) -> Result<cdk02::Id>;
    async fn sign(&self, qid: Uuid, msg: &cdk00::BlindedMessage) -> Result<cdk00::BlindSignature>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait EBillNode: Sync {
    async fn validate_and_decrypt_shared_bill(
        &self,
        shared_bill: &SharedBill,
    ) -> Result<bcr_wdc_webapi::quotes::BillInfo>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Wallet {
    async fn get_blinds(
        &self,
        kid: cdk02::Id,
        amount: Amount,
    ) -> Result<(Uuid, Vec<cdk00::BlindedMessage>)>;

    async fn store_signatures(
        &self,
        rid: Uuid,
        expire: TStamp,
        signatures: Vec<cdk00::BlindSignature>,
    ) -> Result<()>;
}

// ---------- Service
#[derive(Clone)]
pub struct Service<KeysHndlr, Wlt, QuotesRepo, EBillCl> {
    pub keys_hndlr: KeysHndlr,
    pub quotes: QuotesRepo,
    pub wallet: Wlt,
    pub ebill: EBillCl,
}

impl<KeysHndlr, Wlt, QuotesRepo, EBillCl> Service<KeysHndlr, Wlt, QuotesRepo, EBillCl>
where
    QuotesRepo: Repository,
    EBillCl: EBillNode,
{
    pub(crate) const USER_DECISION_RETENTION: chrono::Duration = chrono::Duration::days(1);

    pub async fn new_quote(
        &self,
        bill: BillInfo,
        pub_key: cdk01::PublicKey,
        submitted: TStamp,
    ) -> Result<Uuid> {
        let quote = Quote::new(bill, pub_key, submitted);
        let qid = quote.id;
        self.quotes.store(quote).await?;
        Ok(qid)
    }

    pub async fn validate_and_decrypt_shared_bill(
        &self,
        shared_bill: &SharedBill,
    ) -> Result<bcr_wdc_webapi::quotes::BillInfo> {
        self.ebill
            .validate_and_decrypt_shared_bill(shared_bill)
            .await
    }

    pub async fn enquire(
        &self,
        bill: BillInfo,
        pub_key: cdk01::PublicKey,
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
            None => self.new_quote(bill, pub_key, submitted).await,
        }
    }

    pub async fn cancel(&self, id: uuid::Uuid, submitted: TStamp) -> Result<()> {
        let old = self.quotes.load(id).await?;
        if old.is_none() {
            return Err(Error::UnknownQuoteID(id));
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
            return Err(Error::UnknownQuoteID(id));
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
            return Err(Error::UnknownQuoteID(id));
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
            return Err(Error::UnknownQuoteID(id));
        }
        let mut quote = old.unwrap();
        quote.accept(submitted)?;
        self.quotes
            .update_status_if_offered(quote.id, quote.status)
            .await?;
        Ok(())
    }

    pub async fn lookup(&self, id: uuid::Uuid, now: TStamp) -> Result<Quote> {
        let mut quote = self
            .quotes
            .load(id)
            .await?
            .ok_or(Error::UnknownQuoteID(id))?;
        let changed = quote.check_expire(now);
        if changed {
            self.quotes
                .update_status_if_offered(quote.id, quote.status.clone())
                .await?;
        }
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
}

impl<KeysHndlr, Wlt, QuotesRepo, EBillCl> Service<KeysHndlr, Wlt, QuotesRepo, EBillCl>
where
    KeysHndlr: KeysHandler,
    Wlt: Wallet,
    QuotesRepo: Repository,
    EBillCl: EBillNode,
{
    pub async fn offer(
        &self,
        qid: uuid::Uuid,
        discounted: Amount,
        submitted: TStamp,
        ttl: Option<TStamp>,
    ) -> Result<(Amount, TStamp)> {
        let mut quote = self.lookup(qid, submitted).await?;
        let Status::Pending { public_key } = quote.status else {
            return Err(Error::QuoteAlreadyResolved(qid));
        };

        let fees = quote.bill.sum - discounted;
        let maturity_date = quote.bill.maturity_date;
        let kid = self
            .keys_hndlr
            .generate(qid, discounted, public_key, maturity_date)
            .await?;

        let (request_id, fees_blinds) = self.wallet.get_blinds(kid, fees).await?;
        let joined: JoinAll<_> = fees_blinds
            .iter()
            .map(|blind| self.keys_hndlr.sign(qid, blind))
            .collect();
        let signatures_fees: Vec<cdk00::BlindSignature> =
            joined.await.into_iter().collect::<Result<_>>()?;

        let expiration = ttl.unwrap_or(calculate_default_expiration_date_for_quote(submitted));

        self.wallet
            .store_signatures(request_id, expiration, signatures_fees)
            .await?;

        quote.offer(kid, expiration, discounted)?;
        self.quotes
            .update_status_if_pending(quote.id, quote.status)
            .await?;

        Ok((discounted, expiration))
    }
}

pub fn calculate_default_expiration_date_for_quote(now: crate::TStamp) -> super::TStamp {
    now + chrono::Duration::days(2)
}

#[cfg(test)]
mod tests {

    use super::*;
    use bcr_ebill_core::contact::{BillIdentParticipant, BillParticipant};
    use bcr_wdc_utils::keys::test_utils as keys_utils;
    use bcr_wdc_webapi::test_utils::{random_bill_id, random_node_id};
    use mockall::predicate::*;
    use rand::{seq::IteratorRandom, Rng};

    fn generate_random_identity() -> bcr_ebill_core::contact::BillIdentParticipant {
        let identities = vec![
            BillIdentParticipant {
                t: bcr_ebill_core::contact::ContactType::Person,
                node_id: random_node_id(),
                name: String::from("Alice"),
                postal_address: bcr_ebill_core::PostalAddress {
                    country: String::from("USA"),
                    city: String::from("New York"),
                    zip: None,
                    address: String::from("123 Main St"),
                },
                email: None,
                nostr_relays: vec![],
            },
            BillIdentParticipant {
                t: bcr_ebill_core::contact::ContactType::Company,
                node_id: random_node_id(),
                name: String::from("Bob Corp"),
                postal_address: bcr_ebill_core::PostalAddress {
                    country: String::from("UK"),
                    city: String::from("London"),
                    zip: None,
                    address: String::from("456 High St"),
                },
                email: None,
                nostr_relays: vec![],
            },
            BillIdentParticipant {
                t: bcr_ebill_core::contact::ContactType::Person,
                node_id: random_node_id(),
                name: String::from("Charlie"),
                postal_address: bcr_ebill_core::PostalAddress {
                    country: String::from("France"),
                    city: String::from("Paris"),
                    zip: None,
                    address: String::from("789 Rue de Paris"),
                },
                email: None,
                nostr_relays: vec![],
            },
            BillIdentParticipant {
                t: bcr_ebill_core::contact::ContactType::Company,
                node_id: random_node_id(),
                name: String::from("Dave Ltd"),
                postal_address: bcr_ebill_core::PostalAddress {
                    country: String::from("Japan"),
                    city: String::from("Tokyo"),
                    zip: None,
                    address: String::from("101 Shibuya St"),
                },
                email: None,
                nostr_relays: vec![],
            },
            BillIdentParticipant {
                t: bcr_ebill_core::contact::ContactType::Person,
                node_id: random_node_id(),
                name: String::from("Eve"),
                postal_address: bcr_ebill_core::PostalAddress {
                    country: String::from("Germany"),
                    city: String::from("Berlin"),
                    zip: None,
                    address: String::from("555 Alexanderplatz"),
                },
                email: None,
                nostr_relays: vec![],
            },
        ];
        let mut rng = rand::thread_rng();
        identities.into_iter().choose(&mut rng).unwrap().clone()
    }

    fn generate_random_bill() -> BillInfo {
        let mut rng = rand::thread_rng();
        let holder = generate_random_identity();
        BillInfo {
            id: random_bill_id(),
            drawee: generate_random_identity(),
            drawer: generate_random_identity(),
            payee: BillParticipant::Ident(holder.clone()),
            current_holder: BillParticipant::Ident(holder),
            endorsees: Default::default(),
            sum: Amount::from_sat(rng.gen_range(1000..100000)),
            maturity_date: chrono::Utc::now() + chrono::Duration::days(rng.gen_range(10..30)),
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
        let wallet = MockWallet::new();
        let ebill = MockEBillNode::new();

        let rnd_bill = generate_random_bill();
        let service = Service {
            quotes,
            keys_hndlr,
            wallet,
            ebill,
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
        let public_key = keys_utils::publics()[0];
        let mut repo = MockRepository::new();
        let cloned = rnd_bill.clone();
        repo.expect_search_by_bill()
            .with(
                eq(rnd_bill.id.clone()),
                eq(rnd_bill.payee.node_id().clone()),
            )
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: Status::Pending { public_key },
                    id,
                    bill: cloned.clone(),
                    submitted: chrono::Utc::now(),
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let keys_hndlr = MockKeysHandler::new();
        let wallet = MockWallet::new();
        let ebill = MockEBillNode::new();

        let service = Service {
            quotes: repo,
            keys_hndlr,
            wallet,
            ebill,
        };
        let test_id = service
            .enquire(rnd_bill, public_key, chrono::Utc::now())
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
        let wallet = MockWallet::new();
        let ebill = MockEBillNode::new();

        let service = Service {
            quotes: repo,
            keys_hndlr,
            wallet,
            ebill,
        };
        let test_id = service.enquire(rnd_bill, public_key, now).await.unwrap();
        assert_eq!(id, test_id);
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_offered() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let keyset_id = keys_utils::generate_random_keysetid();
        let public_key = keys_utils::publics()[0];
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
                    },
                    id,
                    bill: cloned.clone(),
                    submitted: now,
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let keys_hndlr = MockKeysHandler::new();
        let wallet = MockWallet::new();
        let ebill = MockEBillNode::new();

        let service = Service {
            quotes: repo,
            keys_hndlr,
            wallet,
            ebill,
        };
        let test_id = service.enquire(rnd_bill, public_key, now).await.unwrap();
        assert_eq!(id, test_id);
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_offered_but_expired() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let cloned = rnd_bill.clone();
        let keyset_id = keys_utils::generate_random_keysetid();
        let public_key = keys_utils::publics()[0];
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
                    },
                    id,
                    bill: cloned.clone(),
                    submitted: now,
                }])
            });
        repo.expect_update_status_if_offered()
            .returning(|_, _| Ok(()));
        let keys_hndlr = MockKeysHandler::new();
        let wallet = MockWallet::new();
        let ebill = MockEBillNode::new();

        let service = Service {
            quotes: repo,
            keys_hndlr,
            wallet,
            ebill,
        };
        let test_id = service
            .enquire(rnd_bill, public_key, now + chrono::Duration::seconds(1))
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
        let public_key = keys_utils::publics()[0];
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
        let wallet = MockWallet::new();
        let ebill = MockEBillNode::new();

        let service = Service {
            quotes: repo,
            keys_hndlr,
            wallet,
            ebill,
        };
        let submitted = now
            + Service::<MockKeysHandler, MockWallet, MockRepository, MockEBillNode>::USER_DECISION_RETENTION
            + chrono::Duration::seconds(1);
        let test_id = service.enquire(rnd_bill, public_key, submitted).await;
        assert!(test_id.is_ok());
        assert_ne!(id, test_id.unwrap());
    }
}
