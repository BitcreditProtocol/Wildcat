// ----- standard library imports
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bcr_wdc_webapi::bill::NodeId;
use bitcoin::Amount;
use cashu::{nut00 as cdk00, nut01 as cdk01, nut02 as cdk02};
use futures::future::JoinAll;
use uuid::Uuid;
// ----- local imports
use crate::error::{Error, Result};
use crate::quotes::{BillInfo, LightQuote, Quote, QuoteStatus, QuoteStatusDiscriminants};
use crate::TStamp;

// ---------- required traits
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ListFilters {
    pub bill_maturity_date_from: Option<chrono::NaiveDate>,
    pub bill_maturity_date_to: Option<chrono::NaiveDate>,
    pub status: Option<QuoteStatusDiscriminants>,
    pub bill_drawee_id: Option<String>,
    pub bill_drawer_id: Option<String>,
    pub bill_payer_id: Option<String>,
    pub bill_holder_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum SortOrder {
    BillMaturityDateAsc,
    BillMaturityDateDesc,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository {
    async fn load(&self, id: uuid::Uuid) -> AnyResult<Option<Quote>>;
    async fn update_status_if_pending(&self, id: uuid::Uuid, quote: QuoteStatus) -> AnyResult<()>;
    async fn update_status_if_offered(&self, id: uuid::Uuid, quote: QuoteStatus) -> AnyResult<()>;
    async fn list_pendings(&self, since: Option<TStamp>) -> AnyResult<Vec<Uuid>>;
    async fn list_light(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
    ) -> AnyResult<Vec<LightQuote>>;
    async fn search_by_bill(&self, bill: &str, endorser: &str) -> AnyResult<Vec<Quote>>;
    async fn store(&self, quote: Quote) -> AnyResult<()>;
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
pub struct Service<KeysHndlr, Wlt, QuotesRepo> {
    pub keys_hndlr: KeysHndlr,
    pub quotes: QuotesRepo,
    pub wallet: Wlt,
}

impl<KeysHndlr, Wlt, QuotesRepo> Service<KeysHndlr, Wlt, QuotesRepo>
where
    QuotesRepo: Repository,
{
    const REJECTION_RETENTION: chrono::Duration = chrono::Duration::days(1);

    pub async fn enquire(
        &self,
        bill: BillInfo,
        pub_key: cdk01::PublicKey,
        submitted: TStamp,
    ) -> Result<uuid::Uuid> {
        let holder_id = &bill.endorsees.last().unwrap_or(&bill.payee).node_id();
        let mut quotes = self
            .quotes
            .search_by_bill(&bill.id, holder_id)
            .await
            .map_err(Error::QuotesRepository)?;

        // pick the more recent quote for this eBill/endorser
        quotes.sort_by_key(|q| q.submitted);
        // user rejected the offer recently
        match quotes.last() {
            Some(Quote {
                id,
                status: QuoteStatus::Pending { .. },
                ..
            }) => Ok(*id),
            Some(Quote {
                id,
                status: QuoteStatus::Denied,
                ..
            }) => Err(Error::QuoteAlreadyResolved(*id)),
            Some(Quote {
                id,
                status: QuoteStatus::Offered { ttl, .. },
                ..
            }) => {
                if *ttl < submitted {
                    let quote = Quote::new(bill, pub_key, submitted);
                    let id = quote.id;
                    self.quotes
                        .store(quote)
                        .await
                        .map_err(Error::QuotesRepository)?;
                    Ok(id)
                } else {
                    Err(Error::QuoteAlreadyResolved(*id))
                }
            }
            Some(Quote {
                id,
                status: QuoteStatus::Accepted { .. },
                ..
            }) => Err(Error::QuoteAlreadyResolved(*id)),
            Some(Quote {
                id,
                status: QuoteStatus::Rejected { tstamp },
                ..
            }) => {
                if (submitted - tstamp) > Self::REJECTION_RETENTION {
                    let quote = Quote::new(bill, pub_key, submitted);
                    let id = quote.id;
                    self.quotes
                        .store(quote)
                        .await
                        .map_err(Error::QuotesRepository)?;
                    Ok(id)
                } else {
                    Err(Error::QuoteAlreadyResolved(*id))
                }
            }
            None => {
                let quote = Quote::new(bill, pub_key, submitted);
                let id = quote.id;
                self.quotes
                    .store(quote)
                    .await
                    .map_err(Error::QuotesRepository)?;
                Ok(id)
            }
        }
    }

    pub async fn deny(&self, id: uuid::Uuid) -> Result<()> {
        let old = self
            .quotes
            .load(id)
            .await
            .map_err(Error::QuotesRepository)?;
        if old.is_none() {
            return Err(Error::UnknownQuoteID(id));
        }
        let mut quote = old.unwrap();
        quote.deny()?;
        self.quotes
            .update_status_if_pending(quote.id, quote.status)
            .await
            .map_err(Error::QuotesRepository)?;
        Ok(())
    }

    pub async fn reject(&self, id: uuid::Uuid, tstamp: TStamp) -> Result<()> {
        let old = self
            .quotes
            .load(id)
            .await
            .map_err(Error::QuotesRepository)?;
        if old.is_none() {
            return Err(Error::UnknownQuoteID(id));
        }
        let mut quote = old.unwrap();
        quote.reject(tstamp)?;
        self.quotes
            .update_status_if_offered(quote.id, quote.status)
            .await
            .map_err(Error::QuotesRepository)?;
        Ok(())
    }

    pub async fn accept(&self, id: uuid::Uuid) -> Result<()> {
        let old = self
            .quotes
            .load(id)
            .await
            .map_err(Error::QuotesRepository)?;
        if old.is_none() {
            return Err(Error::UnknownQuoteID(id));
        }
        let mut quote = old.unwrap();
        quote.accept()?;
        self.quotes
            .update_status_if_offered(quote.id, quote.status)
            .await
            .map_err(Error::QuotesRepository)?;
        Ok(())
    }

    pub async fn lookup(&self, id: uuid::Uuid) -> Result<Quote> {
        self.quotes
            .load(id)
            .await
            .map_err(Error::QuotesRepository)?
            .ok_or(Error::UnknownQuoteID(id))
    }

    pub async fn list_pendings(&self, since: Option<TStamp>) -> Result<Vec<uuid::Uuid>> {
        self.quotes
            .list_pendings(since)
            .await
            .map_err(Error::QuotesRepository)
    }

    pub async fn list_light(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
    ) -> Result<Vec<LightQuote>> {
        self.quotes
            .list_light(filters, sort)
            .await
            .map_err(Error::QuotesRepository)
    }
}

impl<KeysHndlr, Wlt, QuotesRepo> Service<KeysHndlr, Wlt, QuotesRepo>
where
    KeysHndlr: KeysHandler,
    Wlt: Wallet,
    QuotesRepo: Repository,
{
    pub async fn offer(
        &self,
        qid: uuid::Uuid,
        discounted: Amount,
        now: TStamp,
        ttl: Option<TStamp>,
    ) -> Result<(Amount, TStamp)> {
        let mut quote = self.lookup(qid).await?;
        let QuoteStatus::Pending { public_key } = quote.status else {
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

        let expiration = ttl.unwrap_or(calculate_default_expiration_date_for_quote(now));

        self.wallet
            .store_signatures(request_id, expiration, signatures_fees)
            .await?;

        quote.offer(kid, expiration)?;
        self.quotes
            .update_status_if_pending(quote.id, quote.status)
            .await
            .map_err(Error::QuotesRepository)?;

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
    use mockall::predicate::*;
    use rand::{seq::IteratorRandom, Rng};

    fn generate_random_identity() -> bcr_ebill_core::contact::BillIdentParticipant {
        let identities = vec![
            BillIdentParticipant {
                t: bcr_ebill_core::contact::ContactType::Person,
                node_id: String::from(
                    "02a5b1c2d3e4f56789abcdef0123456789abcdef0123456789abcdef0123456789",
                ),
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
                node_id: String::from(
                    "03b2c3d4e5f6789abcdef0123456789abcdef0123456789abcdef0123456789",
                ),
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
                node_id: String::from(
                    "02c3d4e5f6789abcdef0123456789abcdef0123456789abcdef0123456789",
                ),
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
                node_id: String::from(
                    "03d4e5f6789abcdef0123456789abcdef0123456789abcdef0123456789",
                ),
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
                node_id: String::from("02e5f6789abcdef0123456789abcdef0123456789abcdef0123456789"),
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
        let ids = keys_utils::publics();
        let holder = generate_random_identity();
        BillInfo {
            id: ids.into_iter().choose(&mut rng).unwrap().to_string(),
            drawee: generate_random_identity(),
            drawer: generate_random_identity(),
            payee: BillParticipant::Ident(holder.clone()),
            current_holder: BillParticipant::Ident(holder),
            endorsees: Default::default(),
            sum: Amount::from_sat(rng.gen_range(1000..100000)),
            maturity_date: chrono::Utc::now() + chrono::Duration::days(rng.gen_range(10..30)),
        }
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_not_present() {
        let mut quotes = MockRepository::new();
        quotes.expect_search_by_bill().returning(|_, _| Ok(vec![]));
        quotes.expect_store().returning(|_| Ok(()));
        let keys_hndlr = MockKeysHandler::new();
        let wallet = MockWallet::new();

        let rnd_bill = generate_random_bill();
        let service = Service {
            quotes,
            keys_hndlr,
            wallet,
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
                    status: QuoteStatus::Pending { public_key },
                    id,
                    bill: cloned.clone(),
                    submitted: chrono::Utc::now(),
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let keys_hndlr = MockKeysHandler::new();
        let wallet = MockWallet::new();

        let service = Service {
            quotes: repo,
            keys_hndlr,
            wallet,
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
        let mut repo = MockRepository::new();
        repo.expect_search_by_bill()
            .with(
                eq(rnd_bill.id.clone()),
                eq(rnd_bill.payee.node_id().clone()),
            )
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: QuoteStatus::Denied,
                    id,
                    bill: cloned.clone(),
                    submitted: chrono::Utc::now(),
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let keys_hndlr = MockKeysHandler::new();
        let wallet = MockWallet::new();

        let service = Service {
            quotes: repo,
            keys_hndlr,
            wallet,
        };
        let test_id = service
            .enquire(rnd_bill, public_key, chrono::Utc::now())
            .await;
        assert!(test_id.is_err());
        assert!(matches!(
            test_id.unwrap_err(),
            Error::QuoteAlreadyResolved(_)
        ));
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_offered() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let keyset_id = keys_utils::generate_random_keysetid();
        let public_key = keys_utils::publics()[0];
        let cloned = rnd_bill.clone();
        let mut repo = MockRepository::new();
        repo.expect_search_by_bill()
            .with(
                eq(rnd_bill.id.clone()),
                eq(rnd_bill.payee.node_id().clone()),
            )
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: QuoteStatus::Offered {
                        keyset_id,
                        ttl: chrono::Utc::now() + chrono::Duration::days(1),
                    },
                    id,
                    bill: cloned.clone(),
                    submitted: chrono::Utc::now(),
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let keys_hndlr = MockKeysHandler::new();
        let wallet = MockWallet::new();

        let service = Service {
            quotes: repo,
            keys_hndlr,
            wallet,
        };
        let test_id = service
            .enquire(rnd_bill, public_key, chrono::Utc::now())
            .await;
        assert!(test_id.is_err());
        assert!(matches!(
            test_id.unwrap_err(),
            Error::QuoteAlreadyResolved(_)
        ));
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_offered_but_expired() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let cloned = rnd_bill.clone();
        let keyset_id = keys_utils::generate_random_keysetid();
        let public_key = keys_utils::publics()[0];
        let mut repo = MockRepository::new();
        repo.expect_search_by_bill()
            .with(
                eq(rnd_bill.id.clone()),
                eq(rnd_bill.payee.node_id().clone()),
            )
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: QuoteStatus::Offered {
                        keyset_id,
                        ttl: chrono::Utc::now(),
                    },
                    id,
                    bill: cloned.clone(),
                    submitted: chrono::Utc::now(),
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let keys_hndlr = MockKeysHandler::new();
        let wallet = MockWallet::new();

        let service = Service {
            quotes: repo,
            keys_hndlr,
            wallet,
        };
        let test_id = service
            .enquire(
                rnd_bill,
                public_key,
                chrono::Utc::now() + chrono::Duration::seconds(1),
            )
            .await;
        assert!(test_id.is_ok());
        assert_ne!(id, test_id.unwrap());
    }
}
