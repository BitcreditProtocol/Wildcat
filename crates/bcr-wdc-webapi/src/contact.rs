use bcr_ebill_core::contact;
// ----- standard library imports
// ----- extra library imports
use borsh::{BorshDeserialize, BorshSerialize};
use utoipa::ToSchema;
// ----- local modules
// ----- end imports

#[repr(u8)]
#[derive(
    Debug,
    Default,
    Copy,
    Clone,
    serde_repr::Serialize_repr,
    serde_repr::Deserialize_repr,
    PartialEq,
    Eq,
    BorshSerialize,
    BorshDeserialize,
    ToSchema,
)]
#[borsh(use_discriminant = true)]
pub enum ContactType {
    #[default]
    Person = 0,
    Company = 1,
    Anon = 2,
}

impl TryFrom<u64> for ContactType {
    type Error = bcr_ebill_core::ValidationError;

    fn try_from(value: u64) -> std::result::Result<Self, Self::Error> {
        Ok(contact::ContactType::try_from(value)?.into())
    }
}

impl From<contact::ContactType> for ContactType {
    fn from(val: contact::ContactType) -> Self {
        match val {
            contact::ContactType::Person => ContactType::Person,
            contact::ContactType::Company => ContactType::Company,
            contact::ContactType::Anon => ContactType::Anon,
        }
    }
}

impl From<ContactType> for contact::ContactType {
    fn from(value: ContactType) -> Self {
        match value {
            ContactType::Person => contact::ContactType::Person,
            ContactType::Company => contact::ContactType::Company,
            ContactType::Anon => contact::ContactType::Anon,
        }
    }
}
