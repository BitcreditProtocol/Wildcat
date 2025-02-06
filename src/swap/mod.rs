// ----- standard library imports
// ----- extra library imports
// ----- local modules
mod error;
mod service;
pub mod web;
// ----- local imports
pub use service::KeysRepository;
pub use service::ProofRepository;
pub use service::Service;
