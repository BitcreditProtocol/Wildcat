// ----- standard library imports
// ----- extra library imports
use anyhow::{Error as AnyError, Result as AnyResult};
use async_trait::async_trait;
use bcr_wdc_keys as keys;
use bcr_wdc_keys::KeysetID;
use cdk::nuts::nut00 as cdk00;
use cdk::nuts::nut02 as cdk02;
use rust_decimal::{prelude::ToPrimitive, Decimal};
use thiserror::Error;
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::keys::{sign_with_keys, Result as KeyResult};
use crate::utils;
use crate::TStamp;

// ----- error
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    // external errors wrappers
    #[error("keys error {0}")]
    Keys(#[from] crate::keys::Error),
    #[error("credit::keys error {0}")]
    CreditKeys(#[from] crate::credit::keys::Error),
    #[error("quotes repository error {0}")]
    Repository(#[from] AnyError),

    #[error("Quote has been already resolved: {0}")]
    QuoteAlreadyResolved(uuid::Uuid),
    #[error("unknown quote id {0}")]
    UnknownQuoteID(uuid::Uuid),
    #[error("Invalid amount: {0}")]
    InvalidAmount(rust_decimal::Decimal),
}

#[derive(Debug, Clone)]
pub enum QuoteStatus {
    Pending {
        blinds: Vec<cdk00::BlindedMessage>,
    },
    Denied,
    Offered {
        signatures: Vec<cdk00::BlindSignature>,
        ttl: TStamp,
    },
    Rejected {
        tstamp: TStamp,
    },
    Accepted {
        signatures: Vec<cdk00::BlindSignature>,
    },
}

#[derive(Debug, Clone)]
pub struct Quote {
    pub status: QuoteStatus,
    pub id: Uuid,
    pub bill: String,
    pub endorser: String,
    pub submitted: TStamp,
}

impl Quote {
    pub fn new(
        bill: String,
        endorser: String,
        blinds: Vec<cdk00::BlindedMessage>,
        submitted: TStamp,
    ) -> Self {
        Self {
            status: QuoteStatus::Pending { blinds },
            id: Uuid::new_v4(),
            bill,
            endorser,
            submitted,
        }
    }

    pub fn deny(&mut self) -> Result<()> {
        if let QuoteStatus::Pending { .. } = self.status {
            self.status = QuoteStatus::Denied;
            Ok(())
        } else {
            Err(Error::QuoteAlreadyResolved(self.id))
        }
    }

    pub fn offer(&mut self, signatures: Vec<cdk00::BlindSignature>, ttl: TStamp) -> Result<()> {
        let QuoteStatus::Pending { .. } = self.status else {
            return Err(Error::QuoteAlreadyResolved(self.id));
        };

        self.status = QuoteStatus::Offered { signatures, ttl };
        Ok(())
    }

    pub fn reject(&mut self, tstamp: TStamp) -> Result<()> {
        if let QuoteStatus::Offered { .. } = self.status {
            self.status = QuoteStatus::Rejected { tstamp };
            Ok(())
        } else {
            Err(Error::QuoteAlreadyResolved(self.id))
        }
    }

    pub fn accept(&mut self) -> Result<()> {
        if let QuoteStatus::Offered { signatures, .. } = &self.status {
            self.status = QuoteStatus::Accepted {
                signatures: signatures.clone(),
            };
            Ok(())
        } else {
            Err(Error::QuoteAlreadyResolved(self.id))
        }
    }
}

// ---------- required traits
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository: Send + Sync {
    async fn load(&self, id: uuid::Uuid) -> AnyResult<Option<Quote>>;
    async fn update_if_pending(&self, quote: Quote) -> AnyResult<()>;
    async fn update_if_offered(&self, quote: Quote) -> AnyResult<()>;
    async fn list_pendings(&self, since: Option<TStamp>) -> AnyResult<Vec<Uuid>>;
    async fn list_offers(&self, since: Option<TStamp>) -> AnyResult<Vec<Uuid>>;
    async fn search_by_bill(&self, bill: &str, endorser: &str) -> AnyResult<Vec<Quote>>;
    async fn store(&self, quote: Quote) -> AnyResult<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeyFactory: Send + Sync {
    async fn generate(
        &self,
        kid: KeysetID,
        qid: Uuid,
        maturity_date: TStamp,
    ) -> AnyResult<cdk02::MintKeySet>;
}

// ---------- Service
#[derive(Clone)]
pub struct Service<KeysGen, QuotesRepo> {
    pub keys_gen: KeysGen,
    pub quotes: QuotesRepo,
}

impl<KeysGen, QuotesRepo> Service<KeysGen, QuotesRepo>
where
    QuotesRepo: Repository,
{
    const REJECTION_RETENTION: chrono::Duration = chrono::Duration::days(1);

    pub async fn enquire(
        &self,
        bill: String,
        endorser: String,
        submitted: TStamp,
        blinds: Vec<cdk00::BlindedMessage>,
    ) -> Result<uuid::Uuid> {
        let mut quotes = self.quotes.search_by_bill(&bill, &endorser).await?;

        // pick the more recent quote for this eBill/endorser
        quotes.sort_by_key(|q| std::cmp::Reverse(q.submitted));
        // user rejected the offer recently
        match quotes.first() {
            Some(Quote {
                id,
                status: QuoteStatus::Pending { .. },
                ..
            }) => Ok(*id),
            Some(Quote {
                id,
                status: QuoteStatus::Denied { .. },
                ..
            }) => Err(Error::QuoteAlreadyResolved(*id)),
            Some(Quote {
                id,
                status: QuoteStatus::Offered { ttl, .. },
                ..
            }) => {
                if *ttl < submitted {
                    let quote = Quote::new(bill, endorser, blinds, submitted);
                    let id = quote.id;
                    self.quotes.store(quote).await?;
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
                    let quote = Quote::new(bill, endorser, blinds, submitted);
                    let id = quote.id;
                    self.quotes.store(quote).await?;
                    Ok(id)
                } else {
                    Err(Error::QuoteAlreadyResolved(*id))
                }
            }
            None => {
                let quote = Quote::new(bill, endorser, blinds, submitted);
                let id = quote.id;
                self.quotes.store(quote).await?;
                Ok(id)
            }
        }
    }

    pub async fn deny(&self, id: uuid::Uuid) -> Result<()> {
        let old = self.quotes.load(id).await?;
        if old.is_none() {
            return Err(Error::UnknownQuoteID(id));
        }
        let mut quote = old.unwrap();
        quote.deny()?;
        self.quotes.update_if_pending(quote).await?;
        Ok(())
    }

    pub async fn reject(&self, id: uuid::Uuid, tstamp: TStamp) -> Result<()> {
        let old = self.quotes.load(id).await?;
        if old.is_none() {
            return Err(Error::UnknownQuoteID(id));
        }
        let mut quote = old.unwrap();
        quote.reject(tstamp)?;
        self.quotes.update_if_offered(quote).await?;
        Ok(())
    }

    pub async fn accept(&self, id: uuid::Uuid) -> Result<()> {
        let old = self.quotes.load(id).await?;
        if old.is_none() {
            return Err(Error::UnknownQuoteID(id));
        }
        let mut quote = old.unwrap();
        quote.accept()?;
        self.quotes.update_if_offered(quote).await?;
        Ok(())
    }

    pub async fn lookup(&self, id: uuid::Uuid) -> Result<Quote> {
        self.quotes.load(id).await?.ok_or(Error::UnknownQuoteID(id))
    }

    pub async fn list_pendings(&self, since: Option<TStamp>) -> Result<Vec<uuid::Uuid>> {
        self.quotes
            .list_pendings(since)
            .await
            .map_err(Error::Repository)
    }

    pub async fn list_offers(&self, since: Option<TStamp>) -> Result<Vec<uuid::Uuid>> {
        self.quotes
            .list_offers(since)
            .await
            .map_err(Error::Repository)
    }
}

impl<KeysGen, QuotesRepo> Service<KeysGen, QuotesRepo>
where
    KeysGen: KeyFactory,
    QuotesRepo: Repository,
{
    pub async fn offer(
        &self,
        id: uuid::Uuid,
        discount: Decimal,
        now: TStamp,
        ttl: Option<TStamp>,
    ) -> Result<()> {
        let discounted_amount =
            cdk::Amount::from(discount.to_u64().ok_or(Error::InvalidAmount(discount))?);

        let mut quote = self.lookup(id).await?;
        let qid = quote.id;
        let kid = keys::credit::generate_keyset_id_from_bill(&quote.bill, &quote.endorser);
        let QuoteStatus::Pending { ref mut blinds } = quote.status else {
            return Err(Error::QuoteAlreadyResolved(qid));
        };

        let selected_blinds = utils::select_blinds_to_target(discounted_amount, blinds);
        log::warn!("WARNING: we are leaving fees on the table, ... but we don't know how much (eBill data missing)");

        // TODO! maturity date should come from the eBill
        let maturity_date = now + chrono::Duration::days(30);
        let keyset = self.keys_gen.generate(kid, qid, maturity_date).await?;

        let signatures = selected_blinds
            .iter()
            .map(|blind| sign_with_keys(&keyset, blind))
            .collect::<KeyResult<Vec<cdk00::BlindSignature>>>()?;
        let expiration = ttl.unwrap_or(utils::calculate_default_expiration_date_for_quote(now));
        quote.offer(signatures, expiration)?;
        self.quotes.update_if_pending(quote).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use mockall::predicate::*;

    #[tokio::test]
    async fn test_new_quote_request_quote_not_present() {
        let mut quotes = MockRepository::new();
        quotes.expect_search_by_bill().returning(|_, _| Ok(vec![]));
        quotes.expect_store().returning(|_| Ok(()));
        let keys_gen = MockKeyFactory::new();

        let service = Service { quotes, keys_gen };
        let test = service
            .enquire(
                String::from("billID"),
                String::from("endorserID"),
                chrono::Utc::now(),
                vec![],
            )
            .await;
        assert!(test.is_ok());
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_pending() {
        let id = Uuid::new_v4();
        let bill_id = "billID";
        let endorser_id = "endorserID";
        let mut repo = MockRepository::new();
        repo.expect_search_by_bill()
            .with(eq(String::from(bill_id)), eq(String::from(endorser_id)))
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: QuoteStatus::Pending { blinds: vec![] },
                    id,
                    bill: String::from(bill_id),
                    endorser: String::from(endorser_id),
                    submitted: chrono::Utc::now(),
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let keys_gen = MockKeyFactory::new();

        let service = Service {
            quotes: repo,
            keys_gen,
        };
        let test_id = service
            .enquire(
                String::from(bill_id),
                String::from(endorser_id),
                chrono::Utc::now(),
                vec![],
            )
            .await;
        assert!(test_id.is_ok());
        assert_eq!(id, test_id.unwrap());
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_denied() {
        let id = Uuid::new_v4();
        let bill_id = "billID";
        let endorser_id = "endorserID";
        let mut repo = MockRepository::new();
        repo.expect_search_by_bill()
            .with(eq(String::from(bill_id)), eq(String::from(endorser_id)))
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: QuoteStatus::Denied,
                    id,
                    bill: String::from(bill_id),
                    endorser: String::from(endorser_id),
                    submitted: chrono::Utc::now(),
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let keys_gen = MockKeyFactory::new();

        let service = Service {
            quotes: repo,
            keys_gen,
        };
        let test_id = service
            .enquire(
                String::from(bill_id),
                String::from(endorser_id),
                chrono::Utc::now(),
                vec![],
            )
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
        let bill_id = "billID";
        let endorser_id = "endorserID";
        let mut repo = MockRepository::new();
        repo.expect_search_by_bill()
            .with(eq(String::from(bill_id)), eq(String::from(endorser_id)))
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: QuoteStatus::Offered {
                        signatures: vec![],
                        ttl: chrono::Utc::now() + chrono::Duration::days(1),
                    },
                    id,
                    bill: String::from(bill_id),
                    endorser: String::from(endorser_id),
                    submitted: chrono::Utc::now(),
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let keys_gen = MockKeyFactory::new();

        let service = Service {
            quotes: repo,
            keys_gen,
        };
        let test_id = service
            .enquire(
                String::from(bill_id),
                String::from(endorser_id),
                chrono::Utc::now(),
                vec![],
            )
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
        let bill_id = "billID";
        let endorser_id = "endorserID";
        let mut repo = MockRepository::new();
        repo.expect_search_by_bill()
            .with(eq(String::from(bill_id)), eq(String::from(endorser_id)))
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: QuoteStatus::Offered {
                        signatures: vec![],
                        ttl: chrono::Utc::now(),
                    },
                    id,
                    bill: String::from(bill_id),
                    endorser: String::from(endorser_id),
                    submitted: chrono::Utc::now(),
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let keys_gen = MockKeyFactory::new();

        let service = Service {
            quotes: repo,
            keys_gen,
        };
        let test_id = service
            .enquire(
                String::from(bill_id),
                String::from(endorser_id),
                chrono::Utc::now() + chrono::Duration::seconds(1),
                vec![],
            )
            .await;
        assert!(test_id.is_ok());
        assert_ne!(id, test_id.unwrap());
    }
}
