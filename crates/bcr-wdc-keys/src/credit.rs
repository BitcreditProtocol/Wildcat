// ----- standard library imports
// ----- extra library imports
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;
use cashu::nuts::nut02 as cdk02;
// ----- local imports
use crate::id::KeysetID;

pub fn generate_keyset_id_from_bill(bill: &str, node: &str) -> KeysetID {
    let input = format!("{}{}", bill, node);
    let digest = Sha256::hash(input.as_bytes());
    KeysetID {
        version: cdk02::KeySetVersion::Version00,
        id: digest.as_byte_array()[0..KeysetID::BYTELEN]
            .try_into()
            .expect("cdk::KeysetID BYTELEN == 7"),
    }
}
