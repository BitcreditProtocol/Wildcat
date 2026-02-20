// ----- standard library imports
// ----- extra library imports
// ----- local modules
#[cfg(feature = "auth")]
pub mod client;
pub mod convert;
pub mod info;
pub mod keys;
pub mod signatures;
pub mod surreal;
pub mod built_info {
    // The file has been placed there by the build script.
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}
// ----- local imports

// ----- end imports

pub use crate::keys::KeysetEntry;
