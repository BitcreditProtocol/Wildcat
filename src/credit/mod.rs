// ----- standard library imports
// ----- extra library imports
use cdk::nuts::nut00 as cdk00;
use rust_decimal::{prelude::ToPrimitive, Decimal};
// ----- local modules
pub mod admin;
pub mod error;
pub mod keys;
pub mod persistence;
pub mod quotes;
mod utils;
pub mod web;
// ----- local imports
use error::{Error, Result};

type TStamp = chrono::DateTime<chrono::Utc>;

#[derive(Clone)]
pub struct Controller {
    key_factory: keys::Factory<persistence::InMemoryKeysRepository>,
    keys: persistence::InMemoryKeysRepository,
    quote_factory: quotes::Factory<persistence::InMemoryQuoteRepository>,
    quotes: persistence::InMemoryQuoteRepository,
}

impl Controller {
    pub fn new(
        seed: &[u8],
        quotes: persistence::InMemoryQuoteRepository,
        keys: persistence::InMemoryKeysRepository,
    ) -> Self {
        Self {
            key_factory: keys::Factory::new(seed, keys.clone()),
            quote_factory: quotes::Factory {
                quotes: quotes.clone(),
            },
            keys,
            quotes,
        }
    }
}

impl Controller {
    pub fn enquire(
        &self,
        bill: String,
        endorser: String,
        tstamp: TStamp,
        blinds: Vec<cdk00::BlindedMessage>,
    ) -> Result<uuid::Uuid> {
        Ok(self
            .quote_factory
            .new_quote_request(bill, endorser, blinds, tstamp))
    }

    pub fn lookup(&self, id: uuid::Uuid) -> Result<quotes::Quote> {
        self.quotes.load(id).ok_or(Error::UnknownQuoteID(id))
    }

    pub fn decline(&self, id: uuid::Uuid) -> Result<()> {
        let old = self.quotes.load(id);
        if old.is_none() {
            return Err(Error::UnknownQuoteID(id));
        }
        let mut quote = old.unwrap();
        quote.decline()?;
        self.quotes.update_if_pending(quote);
        Ok(())
    }

    pub fn accept(
        &self,
        id: uuid::Uuid,
        discount: Decimal,
        now: TStamp,
        ttl: Option<TStamp>,
    ) -> Result<()> {
        let discounted_amount =
            cdk::Amount::from(discount.to_u64().ok_or(Error::InvalidAmount(discount))?);

        let mut quote = self.quotes.load(id).ok_or(Error::UnknownQuoteID(id))?;
        let id = quote.id;
        let kid = keys::KeysetID::new(&quote.bill, &quote.endorser);
        println!("kid: {:?}", kid);
        let quotes::QuoteStatus::Pending { ref mut blinds } = quote.status_as_mut() else {
            return Err(Error::QuoteAlreadyResolved(id));
        };

        let selected_blinds = utils::select_blinds_to_target(discounted_amount, blinds);
        log::warn!("WARNING: we are leaving fees on the table, ... but we don't know how much (eBill service missing)");

        let keyset = self.key_factory.generate(kid, id).map_err(Error::from)?;

        let signatures = selected_blinds
            .iter()
            .map(|blind| keys::sign_with_keys(&keyset, blind))
            .collect::<Vec<_>>()
            .into_iter()
            .collect::<keys::Result<Vec<_>>>()?; //::Vec<_>().into_iter().collect::<Result<Vec<_>>>()?;
        let expiration = ttl.unwrap_or(utils::calculate_default_expiration_date_for_quote(now));
        quote.accept(signatures, expiration)?;
        self.quotes.update_if_pending(quote);
        Ok(())
    }

    pub fn list_pendings(&self, since: Option<TStamp>) -> Result<Vec<uuid::Uuid>> {
        Ok(self.quotes.list_pendings(since))
    }

    pub fn list_accepteds(&self) -> Result<Vec<uuid::Uuid>> {
        Ok(self.quotes.list_accepteds())
    }
}
