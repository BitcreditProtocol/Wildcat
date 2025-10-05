// ----- standard library imports
// ----- extra library imports
use bcr_common::wire::identity as wire_identity;
use bcr_ebill_core::identity;
use thiserror::Error;
// ----- local imports

// ----- end imports

#[derive(Debug, Error)]
pub enum Error {}

pub fn identitytype_wire2ebill(input: wire_identity::IdentityType) -> identity::IdentityType {
    match input {
        wire_identity::IdentityType::Ident => identity::IdentityType::Ident,
        wire_identity::IdentityType::Anon => identity::IdentityType::Anon,
    }
}
