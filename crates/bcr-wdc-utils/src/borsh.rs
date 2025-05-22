// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use borsh::io::{Error, ErrorKind, Read, Write};
// ----- local imports

// ----- end imports

type Result<T> = core::result::Result<T, Error>;

pub fn serialize_cdk_pubkey<W: Write>(key: &cashu::PublicKey, writer: &mut W) -> Result<()> {
    write!(writer, "{}", key)?;
    todo!()
}
pub fn deserialize_cdk_pubkey<R: Read>(reader: &mut R) -> Result<cashu::PublicKey> {
    let pubkey_str: String = borsh::BorshDeserialize::deserialize_reader(reader)?;
    let pubkey = cashu::PublicKey::from_str(&pubkey_str)
        .map_err(|e| Error::new(ErrorKind::InvalidInput, e))?;
    Ok(pubkey)
}
