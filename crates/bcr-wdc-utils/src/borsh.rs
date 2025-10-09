// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use borsh::io::{Error, ErrorKind, Read, Write};
// ----- local imports

// ----- end imports

type Result<T> = core::result::Result<T, Error>;

pub fn serialize_cdk_pubkey<W: Write>(key: &cashu::PublicKey, writer: &mut W) -> Result<()> {
    let pubkey_str = key.to_string();
    borsh::BorshSerialize::serialize(&pubkey_str, writer)?;
    Ok(())
}

pub fn deserialize_cdk_pubkey<R: Read>(reader: &mut R) -> Result<cashu::PublicKey> {
    let pubkey_str: String = borsh::BorshDeserialize::deserialize_reader(reader)?;
    let pubkey = cashu::PublicKey::from_str(&pubkey_str)
        .map_err(|e| Error::new(ErrorKind::InvalidInput, e))?;
    Ok(pubkey)
}

pub fn serialize_vec_url<W: std::io::Write>(vec: &[url::Url], writer: &mut W) -> Result<()> {
    let url_strs: Vec<String> = vec.iter().map(|u| u.to_string()).collect();
    borsh::BorshSerialize::serialize(&url_strs, writer)?;
    Ok(())
}

pub fn deserialize_vec_url<R: std::io::Read>(reader: &mut R) -> Result<Vec<url::Url>> {
    let url_strs: Vec<String> = borsh::BorshDeserialize::deserialize_reader(reader)?;
    url_strs
        .into_iter()
        .map(|s| {
            url::Url::from_str(&s)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        })
        .collect()
}

pub fn serialize_vec_of_strs<T>(vec: &[T], writer: &mut impl Write) -> Result<()>
where
    T: std::fmt::Display,
{
    let strs: Vec<String> = vec.iter().map(|v| v.to_string()).collect();
    borsh::BorshSerialize::serialize(&strs, writer)?;
    Ok(())
}

pub fn deserialize_vec_of_strs<T>(reader: &mut impl Read) -> Result<Vec<T>>
where
    T: std::str::FromStr,
    <T as std::str::FromStr>::Err: Sync + Send + std::error::Error + 'static,
{
    let strs: Vec<String> = borsh::BorshDeserialize::deserialize_reader(reader)?;
    strs.into_iter()
        .map(|v| T::from_str(&v))
        .collect::<std::result::Result<Vec<T>, T::Err>>()
        .map_err(|e| Error::new(ErrorKind::InvalidData, e))
}
pub fn serialize_vec_of_jsons<T>(vec: &[T], writer: &mut impl Write) -> Result<()>
where
    T: serde::ser::Serialize,
{
    serde_json::to_writer(writer, vec).map_err(|e| Error::new(ErrorKind::InvalidInput, e))
}

pub fn deserialize_vec_of_jsons<T>(reader: &mut impl Read) -> Result<Vec<T>>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_reader(reader).map_err(|e| Error::new(ErrorKind::InvalidInput, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_deserialize_vec_of_jsons_cdk_proofs() {
        let (_, keyset) = crate::keys::test_utils::generate_random_keyset();
        let amount = cashu::Amount::from_str("1000").unwrap();
        let proofs = crate::signatures::test_utils::generate_proofs(&keyset, &amount.split());
        let mut buf = Vec::new();
        serialize_vec_of_jsons(&proofs, &mut buf).unwrap();
        let deserialized = deserialize_vec_of_jsons(&mut buf.as_slice()).unwrap();
        assert_eq!(proofs, deserialized);
    }

    #[test]
    fn serialize_deserialize_vec_of_strs_cdk_pubkeys() {
        let pks = crate::keys::test_utils::publics();
        let mut buf = Vec::new();
        serialize_vec_of_strs(&pks, &mut buf).unwrap();
        let deserialized = deserialize_vec_of_strs(&mut buf.as_slice()).unwrap();
        assert_eq!(pks, deserialized);
    }
}
