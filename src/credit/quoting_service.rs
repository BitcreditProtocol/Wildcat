// ----- standard library imports
// ----- extra library imports
use anyhow::Result as AnyResult;
use cdk::nuts::nut00 as cdk00;
use cdk::nuts::nut02 as cdk02;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::credit::error::{Error, Result};
use crate::credit::keys::generate_keyset_id_from_bill;
use crate::credit::quotes;
use crate::keys::{sign_with_keys, KeysetID, Result as KeyResult};
use crate::utils;
use crate::TStamp;

pub trait KeyFactory: Send + Sync {
    type Error;
    fn generate(
        &self,
        kid: KeysetID,
        qid: Uuid,
        maturity_date: TStamp,
    ) -> AnyResult<cdk02::MintKeySet>;
}

pub trait QuoteFactory: Send + Sync {
    fn generate(
        &self,
        bill: String,
        endorser: String,
        blinds: Vec<cdk00::BlindedMessage>,
        tstamp: TStamp,
    ) -> AnyResult<Uuid>;
}

pub trait QuoteRepository: Send + Sync {
    fn load(&self, id: uuid::Uuid) -> AnyResult<Option<quotes::Quote>>;
    fn update_if_pending(&self, quote: quotes::Quote) -> AnyResult<()>;
    fn list_pendings(&self, since: Option<TStamp>) -> AnyResult<Vec<Uuid>>;
    fn list_accepteds(&self, since: Option<TStamp>) -> AnyResult<Vec<Uuid>>;
}

#[derive(Clone)]
pub struct QuotingService<KeysGen, QuotesGen, QuotesRepo> {
    pub keys_gen: KeysGen,
    pub quotes_gen: QuotesGen,
    pub quotes: QuotesRepo,
}

impl<KeysGen, QuotesGen, QuotesRepo> QuotingService<KeysGen, QuotesGen, QuotesRepo>
where
    QuotesRepo: QuoteRepository,
{
    pub fn lookup(&self, id: uuid::Uuid) -> Result<quotes::Quote> {
        self.quotes.load(id)?.ok_or(Error::UnknownQuoteID(id))
    }

    pub fn decline(&self, id: uuid::Uuid) -> Result<()> {
        let old = self.quotes.load(id)?;
        if old.is_none() {
            return Err(Error::UnknownQuoteID(id));
        }
        let mut quote = old.unwrap();
        quote.decline()?;
        self.quotes.update_if_pending(quote)?;
        Ok(())
    }

    pub fn list_pendings(&self, since: Option<TStamp>) -> Result<Vec<uuid::Uuid>> {
        self.quotes
            .list_pendings(since)
            .map_err(Error::QuoteRepository)
    }

    pub fn list_accepteds(&self, since: Option<TStamp>) -> Result<Vec<uuid::Uuid>> {
        self.quotes
            .list_accepteds(since)
            .map_err(Error::QuoteRepository)
    }
}

impl<KeysGen, QuotesGen, QuotesRepo> QuotingService<KeysGen, QuotesGen, QuotesRepo>
where
    KeysGen: KeyFactory,
    QuotesRepo: QuoteRepository,
{
    pub fn accept(
        &self,
        id: uuid::Uuid,
        discount: Decimal,
        now: TStamp,
        ttl: Option<TStamp>,
    ) -> Result<()> {
        let discounted_amount =
            cdk::Amount::from(discount.to_u64().ok_or(Error::InvalidAmount(discount))?);

        let mut quote = self.lookup(id)?;
        let qid = quote.id;
        let kid = generate_keyset_id_from_bill(&quote.bill, &quote.endorser);
        let quotes::QuoteStatus::Pending { ref mut blinds } = quote.status else {
            return Err(Error::QuoteAlreadyResolved(qid));
        };

        let selected_blinds = utils::select_blinds_to_target(discounted_amount, blinds);
        log::warn!("WARNING: we are leaving fees on the table, ... but we don't know how much (eBill service missing)");

        // TODO! maturity date should come from the eBill
        let maturity_date = now + chrono::Duration::days(30);
        let keyset = self
            .keys_gen
            .generate(kid, qid, maturity_date)
            .map_err(Error::KeysFactory)?;

        let signatures = selected_blinds
            .iter()
            .map(|blind| sign_with_keys(&keyset, blind))
            .collect::<KeyResult<Vec<cdk00::BlindSignature>>>()?;
        let expiration = ttl.unwrap_or(utils::calculate_default_expiration_date_for_quote(now));
        quote.accept(signatures, expiration)?;
        self.quotes.update_if_pending(quote)?;
        Ok(())
    }
}

impl<KeysGen, QuotesGen, QuotesRepo> QuotingService<KeysGen, QuotesGen, QuotesRepo>
where
    QuotesGen: QuoteFactory,
{
    pub fn enquire(
        &self,
        bill: String,
        endorser: String,
        tstamp: TStamp,
        blinds: Vec<cdk00::BlindedMessage>,
    ) -> Result<uuid::Uuid> {
        self.quotes_gen
            .generate(bill, endorser, blinds, tstamp)
            .map_err(Error::from)
    }
}
