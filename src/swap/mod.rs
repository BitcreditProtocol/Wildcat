// ----- standard library imports
// ----- extra library imports
// ----- local modules
mod error;
mod service;
pub mod web;
// ----- local imports
#[cfg(test)]
pub use service::MockKeysRepository;
pub use service::{KeysRepository, ProofRepository, Service};
