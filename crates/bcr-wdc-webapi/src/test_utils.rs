use std::str::FromStr;

use bcr_ebill_core::{
    bill::BillId,
    blockchain::bill::{
        block::BillIssueBlockData, create_bill_to_share_with_external_party, BillBlockchain,
    },
    util::BcrKeys,
    NodeId, PublicKey,
};
// ----- standard library imports
use chrono::NaiveTime;
// ----- extra library imports
use bcr_wdc_utils::keys::test_utils as keys_test;
use bitcoin::Amount;
use rand::Rng;
// ----- local imports
use crate::{
    bill::BillIdentParticipant,
    contact::ContactType,
    identity::PostalAddress,
    quotes::{EnquireRequest, SharedBill},
};
// ----- end imports

// returns a random `EnquireRequest` with the bill's holder signing keys
pub fn generate_random_bill_enquire_request(
    owner_kp: bitcoin::secp256k1::Keypair,
    payee_kp: Option<bitcoin::secp256k1::Keypair>,
) -> (crate::quotes::EnquireRequest, bitcoin::secp256k1::Keypair) {
    let bill_keys = BcrKeys::from_private_key(&keys_test::generate_random_keypair().secret_key())
        .expect("valid key");
    let bill_id = BillId::new(bill_keys.pub_key(), bitcoin::Network::Testnet);

    let (_, drawee) = random_identity_public_data();
    let (drawer_key_pair, drawer) = random_identity_public_data();
    let (signing_key, payee) = match payee_kp {
        Some(kp) => {
            let mut payee = random_identity_public_data().1;
            payee.node_id = node_id_from_pub_key(kp.public_key());
            (kp, payee)
        }
        None => random_identity_public_data(),
    };
    let (sharer_keys, _) = random_identity_public_data();

    let public_key = owner_kp.public_key();
    let amount = Amount::from_sat(rand::thread_rng().gen_range(1000..100000));

    let core_drawer: bcr_ebill_core::contact::BillIdentParticipant = drawer.into();
    let core_drawee: bcr_ebill_core::contact::BillIdentParticipant = drawee.into();
    let core_payee: bcr_ebill_core::contact::BillIdentParticipant = payee.into();

    let now = chrono::Utc::now().timestamp() as u64;
    let bill_chain = BillBlockchain::new(
        &BillIssueBlockData {
            id: bill_id.clone(),
            country_of_issuing: "AT".to_string(),
            city_of_issuing: "Vienna".to_string(),
            drawee: core_drawee.into(),
            drawer: core_drawer.into(),
            payee: bcr_ebill_core::contact::BillParticipant::Ident(core_payee).into(),
            currency: "sat".to_string(),
            sum: amount.to_sat(),
            maturity_date: random_date(),
            issue_date: random_date(),
            country_of_payment: "AT".to_string(),
            city_of_payment: "Vienna".to_string(),
            language: "en".to_string(),
            files: vec![],
            signatory: None,
            signing_timestamp: now,
            signing_address: bcr_ebill_core::PostalAddress {
                country: "AT".to_string(),
                city: "Vienna".to_string(),
                zip: None,
                address: "Address".to_string(),
            },
        },
        BcrKeys::from_private_key(&drawer_key_pair.secret_key()).expect("valid key"),
        None,
        bill_keys.clone(),
        now,
    )
    .expect("can create bill chain");
    let bill_to_share = create_bill_to_share_with_external_party(
        &bill_id,
        &bill_chain,
        &bcr_ebill_core::bill::BillKeys {
            private_key: bill_keys.get_private_key(),
            public_key: bill_keys.pub_key(),
        },
        &public_key,
        &BcrKeys::from_private_key(&sharer_keys.secret_key()).expect("valid_key"),
        &[],
    )
    .expect("can create sharable bill");

    let shared_bill: SharedBill = bill_to_share.into();

    let request = EnquireRequest {
        content: shared_bill,
        public_key: public_key.into(),
    };
    (request, signing_key)
}

// a hard-coded holder key pair, so both sides (client/server) use the same
pub fn holder_key_pair() -> bitcoin::secp256k1::Keypair {
    bitcoin::secp256k1::Keypair::from_secret_key(
        bitcoin::secp256k1::SECP256K1,
        &bitcoin::secp256k1::SecretKey::from_str(
            "b2f133f2656effb8e807b7d60d9065c269c5e98f5269f7baf05ed8f71eda7e6f",
        )
        .expect("valid key"),
    )
}

pub fn random_bill_id() -> BillId {
    let keypair = keys_test::generate_random_keypair();
    BillId::new(keypair.public_key(), bitcoin::Network::Testnet)
}

pub fn random_node_id() -> NodeId {
    let keypair = keys_test::generate_random_keypair();
    NodeId::new(keypair.public_key(), bitcoin::Network::Testnet)
}

pub fn node_id_from_pub_key(pub_key: PublicKey) -> NodeId {
    NodeId::new(pub_key, bitcoin::Network::Testnet)
}

pub fn random_date() -> String {
    let start = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
        .expect("naivedate")
        .and_time(NaiveTime::from_hms_opt(0, 0, 0).expect("NaiveTime"))
        .and_utc();
    let mut rng = rand::thread_rng();
    let days = chrono::Duration::days(rng.gen_range(0..365));
    let random_date = start + days;
    random_date.to_rfc3339()
}

pub fn random_identity_public_data() -> (bitcoin::secp256k1::Keypair, BillIdentParticipant) {
    let keypair = keys_test::generate_random_keypair();
    let sample = [
        BillIdentParticipant {
            t: ContactType::Person,
            email: Some(String::from("Carissa@kemp.com")),
            name: String::from("Carissa Kemp"),
            node_id: node_id_from_pub_key(keypair.public_key()),
            postal_address: PostalAddress {
                country: String::from("Austria"),
                city: String::from("Vorarlberg"),
                zip: Some(String::from("5196")),
                address: String::from("Auf der Stift 17c"),
            },
            nostr_relays: vec![String::from("")],
        },
        BillIdentParticipant {
            t: ContactType::Person,
            email: Some(String::from("alana@carrillo.com")),
            name: String::from("Alana Carrillo"),
            node_id: node_id_from_pub_key(keypair.public_key()),
            postal_address: PostalAddress {
                country: String::from("Spain"),
                city: String::from("Madrid"),
                zip: Some(String::from("81015")),
                address: String::from("Paseo José Emilio Ruíz 69"),
            },
            nostr_relays: vec![String::from("")],
        },
        BillIdentParticipant {
            t: ContactType::Person,
            email: Some(String::from("geremia@pisani.com")),
            name: String::from("Geremia Pisani"),
            node_id: node_id_from_pub_key(keypair.public_key()),
            postal_address: PostalAddress {
                country: String::from("Italy"),
                city: String::from("Firenze"),
                zip: Some(String::from("50141")),
                address: String::from("Piazza Principale Umberto 148"),
            },
            nostr_relays: vec![String::from("")],
        },
        BillIdentParticipant {
            t: ContactType::Person,
            email: Some(String::from("andreas@koenig.com")),
            name: String::from("Andreas Koenig"),
            node_id: node_id_from_pub_key(keypair.public_key()),
            postal_address: PostalAddress {
                country: String::from("Austria"),
                city: String::from("Lorberhof"),
                zip: Some(String::from("9556")),
                address: String::from("Haiden 96"),
            },
            nostr_relays: vec![String::from("")],
        },
        BillIdentParticipant {
            t: ContactType::Company,
            email: Some(String::from("logistilla@fournier.com")),
            name: String::from("Logistilla Fournier"),
            node_id: node_id_from_pub_key(keypair.public_key()),
            postal_address: PostalAddress {
                country: String::from("France"),
                city: String::from("Toulous"),
                zip: Some(String::from("31000")),
                address: String::from("25, rou Pierre de Coubertin"),
            },
            nostr_relays: vec![String::from("")],
        },
        BillIdentParticipant {
            t: ContactType::Company,
            email: Some(String::from("moonlimited@ltd.com")),
            name: String::from("Moon Limited"),
            node_id: node_id_from_pub_key(keypair.public_key()),
            postal_address: PostalAddress {
                country: String::from("USA"),
                city: String::from("New York"),
                zip: Some(String::from("86659-2593")),
                address: String::from("3443 Joanny Bypass"),
            },
            nostr_relays: vec![String::from("")],
        },
        BillIdentParticipant {
            t: ContactType::Company,
            email: Some(String::from("blanco@spa.com")),
            name: String::from("Blanco y Asoc."),
            node_id: node_id_from_pub_key(keypair.public_key()),
            postal_address: PostalAddress {
                country: String::from("Argentina"),
                city: String::from("Puerto Clara"),
                zip: Some(String::from("38074")),
                address: String::from("Isidora 96 0 7"),
            },
            nostr_relays: vec![String::from("")],
        },
        BillIdentParticipant {
            t: ContactType::Company,
            email: Some(String::from("alexanderurner@grimm.com")),
            name: String::from("Grimm GmbH"),
            node_id: node_id_from_pub_key(keypair.public_key()),
            postal_address: PostalAddress {
                country: String::from("Austria"),
                city: String::from("Perg"),
                zip: Some(String::from("3512")),
                address: String::from("Barthring 342"),
            },
            nostr_relays: vec![String::from("")],
        },
        BillIdentParticipant {
            t: ContactType::Company,
            email: Some(String::from("antoniosegovia@santiago.com")),
            name: String::from("Empresa Santiago"),
            node_id: node_id_from_pub_key(keypair.public_key()),
            postal_address: PostalAddress {
                country: String::from("Spain"),
                city: String::from("Vall Juarez"),
                zip: Some(String::from("88191")),
                address: String::from("Avinguida José Antonio, 20"),
            },
            nostr_relays: vec![String::from("")],
        },
        BillIdentParticipant {
            t: ContactType::Company,
            email: Some(String::from("santoro_group@spa.com")),
            name: String::from("Santoro Group"),
            node_id: node_id_from_pub_key(keypair.public_key()),
            postal_address: PostalAddress {
                country: String::from("Italy"),
                city: String::from("Prunetta"),
                zip: Some(String::from("51020")),
                address: String::from("Corso Vittorio Emanuele, 90"),
            },
            nostr_relays: vec![String::from("")],
        },
    ];

    let mut rng = rand::thread_rng();
    let random_index = rand::Rng::gen_range(&mut rng, 0..sample.len());
    let random_identity = sample[random_index].clone();
    (keypair, random_identity)
}
