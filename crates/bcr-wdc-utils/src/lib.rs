// ----- standard library imports
// ----- extra library imports
// ----- local modules
pub mod attestation;
#[cfg(feature = "auth")]
pub mod client;
pub mod convert;
pub mod db;
pub mod info;
pub mod keys;
pub mod nut19;
pub mod routine;
pub mod signatures;
pub use db::postgres;
pub use db::surreal;
pub mod built_info {
    // The file has been placed there by the build script.
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}
// ----- local imports

// ----- end imports

pub use crate::keys::KeysetEntry;
pub type TStamp = chrono::DateTime<chrono::Utc>;
