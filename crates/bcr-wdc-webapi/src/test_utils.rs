// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use bcr_common::{core::BillId, core_tests, wire::quotes as wire_quotes, wire_tests};
use bcr_ebill_core::protocol::{
    blockchain::bill::{
        block::{BillIssueBlockData, BillSignerIdentityProofBlockdata},
        create_bill_to_share_with_external_party,
        participant::{BillIdentParticipant, BillParticipant},
        BillBlockchain,
    },
    crypto::BcrKeys,
    Address, City, Country, Date, EmailIdentityProofData, PostalAddress, SignedIdentityProof, Sum,
    Timestamp,
};
use bcr_wdc_utils::convert;
use bitcoin::{self as btc, secp256k1 as secp, Amount};
use chrono::NaiveTime;
use rand::Rng;
// ----- local imports

// ----- end imports

// returns a random `EnquireRequest` with the bill's holder signing keys
pub fn generate_random_bill_enquire_request(
    receiver_pk: btc::PublicKey,
    payee_kp: Option<secp::Keypair>,
    amount: Option<btc::Amount>,
) -> (wire_quotes::EnquireRequest, secp::Keypair) {
    let bill_keys = BcrKeys::from_private_key(&core_tests::generate_random_keypair().secret_key());
    let bill_id = BillId::new(bill_keys.pub_key(), btc::Network::Testnet);
    let (_, drawee) = wire_tests::random_identity_public_data();
    let (drawer_key_pair, drawer) = wire_tests::random_identity_public_data();
    let (signing_key, payee) = match payee_kp {
        Some(kp) => {
            let mut payee = wire_tests::random_identity_public_data().1;
            payee.node_id = core_tests::node_id_from_pub_key(kp.public_key());
            (kp, payee)
        }
        None => wire_tests::random_identity_public_data(),
    };
    let default_amount = Amount::from_sat(rand::thread_rng().gen_range(1000..100000));
    let amount = amount.unwrap_or(default_amount);
    let core_drawer: BillIdentParticipant =
        convert::billidentparticipant_wire2ebill(drawer).unwrap();
    let core_drawee: BillIdentParticipant =
        convert::billidentparticipant_wire2ebill(drawee).unwrap();
    let core_payee: BillIdentParticipant = convert::billidentparticipant_wire2ebill(payee).unwrap();
    let now = chrono::Utc::now();
    let created_at = Timestamp::new(10000).unwrap();
    let (id_proof, email_proof) =
        signed_identity_proof_test(core_drawer.clone(), signing_key, created_at);
    let bill_chain = BillBlockchain::new(
        &BillIssueBlockData {
            id: bill_id.clone(),
            country_of_issuing: Country::AT,
            city_of_issuing: City::new("Vienna").unwrap(),
            drawee: core_drawee.into(),
            drawer: core_drawer.into(),
            payee: BillParticipant::Ident(core_payee).into(),
            sum: Sum::new_sat(amount.to_sat()).unwrap(),
            maturity_date: random_date(),
            issue_date: random_date(),
            country_of_payment: Country::AT,
            city_of_payment: City::new("Vienna").unwrap(),
            files: vec![],
            signatory: None,
            signing_timestamp: now.into(),
            signing_address: PostalAddress {
                country: Country::AT,
                city: City::new("Vienna").unwrap(),
                zip: None,
                address: Address::new("Address").unwrap(),
            },
            signer_identity_proof: BillSignerIdentityProofBlockdata {
                node_id: email_proof.node_id,
                company_node_id: None,
                email: email_proof.email,
                created_at: email_proof.created_at,
                signature: id_proof.signature,
                witness: id_proof.witness,
            },
        },
        BcrKeys::from_private_key(&drawer_key_pair.secret_key()),
        None,
        bill_keys.clone(),
        now.into(),
    )
    .expect("can create bill chain");
    let bill_to_share = create_bill_to_share_with_external_party(
        &bill_id,
        &bill_chain,
        &BcrKeys::from_private_key(&bill_keys.get_private_key()),
        &receiver_pk.inner,
        &BcrKeys::from_private_key(&signing_key.secret_key()),
        &[],
    )
    .expect("can create sharable bill");
    let shared_bill: wire_quotes::SharedBill = convert::sharedbill_ebill2wire(bill_to_share);
    let request = wire_quotes::EnquireRequest {
        content: shared_bill,
        minting_pubkey: receiver_pk.inner.into(),
    };
    (request, signing_key)
}

// a hard-coded holder key pair, so both sides (client/server) use the same
pub fn holder_key_pair() -> secp::Keypair {
    secp::Keypair::from_secret_key(
        secp::SECP256K1,
        &secp::SecretKey::from_str(
            "b2f133f2656effb8e807b7d60d9065c269c5e98f5269f7baf05ed8f71eda7e6f",
        )
        .expect("valid key"),
    )
}

pub fn random_date() -> Date {
    let start = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
        .expect("naivedate")
        .and_time(NaiveTime::from_hms_opt(0, 0, 0).expect("NaiveTime"))
        .and_utc();
    let mut rng = rand::thread_rng();
    let days = chrono::Duration::days(rng.gen_range(0..365));
    let random_date = start + days;
    Date::new(random_date.date_naive().to_string()).unwrap()
}

pub fn signed_identity_proof_test(
    identity: BillIdentParticipant,
    signer: secp::Keypair,
    created_at: Timestamp,
) -> (SignedIdentityProof, EmailIdentityProofData) {
    let data = EmailIdentityProofData {
        node_id: identity.node_id.clone(),
        company_node_id: None,
        email: identity.email.unwrap(),
        created_at,
    };
    let proof = data.sign(&identity.node_id, &signer.secret_key()).unwrap();
    (proof, data)
}
