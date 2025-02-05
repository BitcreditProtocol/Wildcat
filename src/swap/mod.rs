// ----- standard library imports
// ----- extra library imports
// ----- local modules
mod error;
mod service;
pub mod web;
// ----- local imports
pub use service::{KeysRepository, ProofRepository, Service};
#[cfg(test)]
pub use service::MockKeysRepository;
