// ----- standard library imports
// ----- extra library imports
use bcr_common::wire::quotes as wire_quotes;
use bcr_wdc_webapi::test_utils::generate_random_bill_enquire_request;
use bitcoin::secp256k1::Keypair;
use cashu::nuts::nut02 as cdk02;
use cashu::Amount;
// ----- local modules
// ----- end imports

pub struct EbillRequestComponents {
    pub bill: wire_quotes::SharedBill,
    pub signing_key: Keypair,
}

pub fn random_ebill_request(
    receiver: bitcoin::PublicKey,
    amount: Option<bitcoin::Amount>,
) -> EbillRequestComponents {
    let (request, signing_key) = generate_random_bill_enquire_request(receiver, None, amount);
    EbillRequestComponents {
        bill: request.content,
        signing_key,
    }
}

pub fn generate_blinds(
    keyset_id: cdk02::Id,
    amounts: &[Amount],
) -> Vec<(
    cashu::BlindedMessage,
    cashu::secret::Secret,
    cashu::SecretKey,
)> {
    let mut blinds = Vec::new();
    for amount in amounts {
        let blind = bcr_wdc_utils::keys::test_utils::generate_blind(keyset_id, *amount);
        blinds.push(blind);
    }
    blinds
}
