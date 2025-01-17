// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
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
    #[error("Quote already exists: {0}")]
    QuoteAlreadyExists(uuid::Uuid),
    #[error("Quote has been already resolved: {0}")]
    QuoteAlreadyResolved(uuid::Uuid),
    #[error("Insufficient blinds")]
    InsufficientBlinds,
    #[error("keys error {0}")]
    Keys(#[from] super::keys::Error),
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
    fn store(&self, quote: Quote);
}

// ---------- Quotes Factory
#[derive(Clone)]
pub struct Factory<Quotes> {
    pub quotes: Quotes,
}

impl<Quotes: Repository> Factory<Quotes> {
    fn add_new(
        &self,
        bill: String,
        endorser: String,
        blinds: Vec<cdk00::BlindedMessage>,
        submitted: TStamp,
    ) -> uuid::Uuid {
        let new = Quote::new(bill, endorser, blinds, submitted);
        let id = new.id;
        self.quotes.store(new);
        id
    }

    pub fn new_quote_request(
        &self,
        bill: String,
        endorser: String,
        blinds: Vec<cdk00::BlindedMessage>,
        submitted: TStamp,
    ) -> uuid::Uuid {
        let Some(quote) = self.quotes.search_by(&bill, &endorser) else {
            let quote = Quote::new(bill, endorser, blinds, submitted);
            let id = quote.id;
            self.quotes.store(quote);
            return id;
        };

        if let QuoteStatus::Accepted { ttl, .. } = quote.status() {
            if *ttl < submitted {
                let new = Quote::new(bill, endorser, blinds, submitted);
                let id = new.id;
                self.quotes.store(new);
                return id;
            }
        }
        quote.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::predicate::*;

    #[test]
    fn test_new_quote_request_quote_not_present() {
        // TODO!
    }

    #[test]
    fn test_new_quote_request_quote_pending() {
        // TODO!
    }

    #[test]
    fn test_new_quote_request_quote_declined() {
        // TODO!
    }

    #[test]
    fn test_new_quote_request_quote_accepted() {
        // TODO!
    }

    #[test]
    fn test_new_quote_request_quote_accepted_but_expired() {
        // TODO!
    }
}
