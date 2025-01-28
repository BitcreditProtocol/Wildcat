// ----- standard library imports
// ----- extra library imports
use bitcoin::bip32 as btc32;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;
use cdk::nuts::nut00 as cdk00;
use thiserror::Error;
use uuid::Uuid;
// ----- local modules
// ----- local imports
use super::TStamp;

// ----- error
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("Quote has been already resolved: {0}")]
    QuoteAlreadyResolved(uuid::Uuid),
    #[error("keys error {0}")]
    Keys(#[from] super::keys::Error),
    #[error("repository error {0}")]
    Repository(#[from] Box<dyn std::error::Error>),
}

pub fn generate_path_idx_from_quoteid(quoteid: Uuid) -> btc32::ChildNumber {
    const MAX_INDEX: u32 = 2_u32.pow(31) - 1;
    let sha_qid = Sha256::hash(quoteid.as_bytes());
    let u_qid = u32::from_be_bytes(sha_qid[0..4].try_into().expect("a u32 is 4 bytes"));
    let idx_qid = std::cmp::min(u_qid, MAX_INDEX);
    btc32::ChildNumber::from_hardened_idx(idx_qid).expect("keyset is a valid index")
}

#[derive(Debug, Clone)]
pub enum QuoteStatus {
    Pending {
        blinds: Vec<cdk00::BlindedMessage>,
    },
    Declined,
    Accepted {
        signatures: Vec<cdk00::BlindSignature>,
        ttl: TStamp,
    },
}

#[derive(Debug, Clone)]
pub struct Quote {
    status: QuoteStatus,
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
            id: uuid::Uuid::new_v4(),
            bill,
            endorser,
            submitted,
        }
    }

    pub fn status(&self) -> &QuoteStatus {
        &self.status
    }

    pub fn status_as_mut(&mut self) -> &mut QuoteStatus {
        &mut self.status
    }

    pub fn decline(&mut self) -> Result<()> {
        if let QuoteStatus::Pending { .. } = self.status {
            self.status = QuoteStatus::Declined;
            Ok(())
        } else {
            Err(Error::QuoteAlreadyResolved(self.id))
        }
    }

    pub fn accept(&mut self, signatures: Vec<cdk00::BlindSignature>, ttl: TStamp) -> Result<()> {
        let QuoteStatus::Pending { .. } = self.status else {
            return Err(Error::QuoteAlreadyResolved(self.id));
        };

        self.status = QuoteStatus::Accepted { signatures, ttl };
        Ok(())
    }
}

// ---------- Quotes Repository
#[cfg_attr(test, mockall::automock)]
pub trait Repository: Send + Sync {
    fn search_by(&self, bill: &str, endorser: &str) -> Option<Quote>;
    fn store(&self, quote: Quote) -> std::result::Result<(), Box<dyn std::error::Error>>;
}

// ---------- Quotes Factory
#[derive(Clone)]
pub struct Factory<Quotes> {
    pub quotes: Quotes,
}

impl<Quotes: Repository> Factory<Quotes> {
    pub fn new_quote_request(
        &self,
        bill: String,
        endorser: String,
        blinds: Vec<cdk00::BlindedMessage>,
        submitted: TStamp,
    ) -> Result<uuid::Uuid> {
        let Some(quote) = self.quotes.search_by(&bill, &endorser) else {
            let quote = Quote::new(bill, endorser, blinds, submitted);
            let id = quote.id;
            self.quotes.store(quote)?;
            return Ok(id);
        };

        if let QuoteStatus::Accepted { ttl, .. } = quote.status() {
            if *ttl < submitted {
                let new = Quote::new(bill, endorser, blinds, submitted);
                let id = new.id;
                self.quotes.store(new)?;
                return Ok(id);
            }
        }
        Ok(quote.id)
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use mockall::predicate::*;

    #[test]
    fn test_new_quote_request_quote_not_present() {
        let mut repo = MockRepository::new();
        repo.expect_search_by()
            .with(eq("billID"), eq("endorserID"))
            .returning(|_, _| None);
        repo.expect_store().returning(|_| Ok(()));

        let factory = Factory { quotes: repo };
        let test = factory.new_quote_request(
            String::from("billID"),
            String::from("endorserID"),
            vec![],
            chrono::Utc::now(),
        );
        assert!(test.is_ok());
    }

    #[test]
    fn test_new_quote_request_quote_pending() {
        let id = Uuid::new_v4();
        let mut repo = MockRepository::new();
        repo.expect_search_by()
            .with(eq("billID"), eq("endorserID"))
            .returning(move |_, _| {
                Some(Quote {
                    status: QuoteStatus::Pending { blinds: vec![] },
                    id,
                    bill: String::from("billID"),
                    endorser: String::from("endorserID"),
                    submitted: chrono::Utc::now(),
                })
            });
        repo.expect_store().returning(|_| Ok(()));

        let factory = Factory { quotes: repo };
        let test_id = factory.new_quote_request(
            String::from("billID"),
            String::from("endorserID"),
            vec![],
            chrono::Utc::now(),
        );
        assert!(test_id.is_ok());
        assert_eq!(id, test_id.unwrap());
    }

    #[test]
    fn test_new_quote_request_quote_declined() {
        let id = Uuid::new_v4();
        let mut repo = MockRepository::new();
        repo.expect_search_by()
            .with(eq("billID"), eq("endorserID"))
            .returning(move |_, _| {
                Some(Quote {
                    status: QuoteStatus::Declined,
                    id,
                    bill: String::from("billID"),
                    endorser: String::from("endorserID"),
                    submitted: chrono::Utc::now(),
                })
            });
        repo.expect_store().returning(|_| Ok(()));

        let factory = Factory { quotes: repo };
        let test_id = factory.new_quote_request(
            String::from("billID"),
            String::from("endorserID"),
            vec![],
            chrono::Utc::now(),
        );
        assert!(test_id.is_ok());
        assert_eq!(id, test_id.unwrap());
    }

    #[test]
    fn test_new_quote_request_quote_accepted() {
        let id = Uuid::new_v4();
        let mut repo = MockRepository::new();
        repo.expect_search_by()
            .with(eq("billID"), eq("endorserID"))
            .returning(move |_, _| {
                Some(Quote {
                    status: QuoteStatus::Accepted {
                        signatures: vec![],
                        ttl: chrono::Utc::now() + chrono::Duration::days(1),
                    },
                    id,
                    bill: String::from("billID"),
                    endorser: String::from("endorserID"),
                    submitted: chrono::Utc::now(),
                })
            });
        repo.expect_store().returning(|_| Ok(()));

        let factory = Factory { quotes: repo };
        let test_id = factory.new_quote_request(
            String::from("billID"),
            String::from("endorserID"),
            vec![],
            chrono::Utc::now(),
        );
        assert!(test_id.is_ok());
        assert_eq!(id, test_id.unwrap());
    }

    #[test]
    fn test_new_quote_request_quote_accepted_but_expired() {
        // TODO!
        let id = Uuid::new_v4();
        let mut repo = MockRepository::new();
        repo.expect_search_by()
            .with(eq("billID"), eq("endorserID"))
            .returning(move |_, _| {
                Some(Quote {
                    status: QuoteStatus::Accepted {
                        signatures: vec![],
                        ttl: chrono::Utc::now(),
                    },
                    id,
                    bill: String::from("billID"),
                    endorser: String::from("endorserID"),
                    submitted: chrono::Utc::now(),
                })
            });
        repo.expect_store().returning(|_| Ok(()));

        let factory = Factory { quotes: repo };
        let test_id = factory.new_quote_request(
            String::from("billID"),
            String::from("endorserID"),
            vec![],
            chrono::Utc::now() + chrono::Duration::seconds(1),
        );
        assert!(test_id.is_ok());
        assert_ne!(id, test_id.unwrap());
    }
}
