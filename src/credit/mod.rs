// ----- standard library imports
// ----- extra library imports
// ----- local modules
pub mod admin;
pub mod error;
pub mod keys;
pub mod persistence;
pub mod quotes;
mod quoting_service;
pub mod web;
// ----- local imports

pub type ProdCreditKeysFactory = keys::Factory<
    persistence::InMemoryQuoteKeysRepository,
    persistence::InMemoryMaturityKeysRepository,
>;
pub type ProdQuoteFactory = quotes::Factory<persistence::InMemoryQuoteRepository>;
pub type ProdQuoteRepository = persistence::InMemoryQuoteRepository;
pub type ProdQuotingService =
    quoting_service::QuotingService<ProdCreditKeysFactory, ProdQuoteFactory, ProdQuoteRepository>;
