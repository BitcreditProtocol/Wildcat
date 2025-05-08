// ----- standard library imports
use chrono::NaiveTime;
// ----- extra library imports
use rand::Rng;
// ----- local imports
use bcr_wdc_utils::keys::test_utils as keys_test;
use crate::quotes::{ContactType, IdentityPublicData, PostalAddress};
// ----- end imports

pub fn random_bill_id() -> String {
    let keypair = keys_test::generate_random_keypair();
    bcr_ebill_core::util::sha256_hash(&keypair.public_key().serialize())
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

pub fn random_identity_public_data() -> (bitcoin::secp256k1::Keypair, IdentityPublicData) {
    let keypair = keys_test::generate_random_keypair();
    let sample = [
        IdentityPublicData {
            t: ContactType::Person,
            email: Some(String::from("Carissa@kemp.com")),
            name: String::from("Carissa Kemp"),
            node_id: keypair.public_key().to_string(),
            postal_address: PostalAddress {
                country: String::from("Austria"),
                city: String::from("Vorarlberg"),
                zip: Some(String::from("5196")),
                address: String::from("Auf der Stift 17c"),
            },
            nostr_relay: Some(String::from("")),
        },
        IdentityPublicData {
            t: ContactType::Person,
            email: Some(String::from("alana@carrillo.com")),
            name: String::from("Alana Carrillo"),
            node_id: keypair.public_key().to_string(),
            postal_address: PostalAddress {
                country: String::from("Spain"),
                city: String::from("Madrid"),
                zip: Some(String::from("81015")),
                address: String::from("Paseo José Emilio Ruíz 69"),
            },
            nostr_relay: Some(String::from("")),
        },
        IdentityPublicData {
            t: ContactType::Person,
            email: Some(String::from("geremia@pisani.com")),
            name: String::from("Geremia Pisani"),
            node_id: keypair.public_key().to_string(),
            postal_address: PostalAddress {
                country: String::from("Italy"),
                city: String::from("Firenze"),
                zip: Some(String::from("50141")),
                address: String::from("Piazza Principale Umberto 148"),
            },
            nostr_relay: Some(String::from("")),
        },
        IdentityPublicData {
            t: ContactType::Person,
            email: Some(String::from("andreas@koenig.com")),
            name: String::from("Andreas Koenig"),
            node_id: keypair.public_key().to_string(),
            postal_address: PostalAddress {
                country: String::from("Austria"),
                city: String::from("Lorberhof"),
                zip: Some(String::from("9556")),
                address: String::from("Haiden 96"),
            },
            nostr_relay: Some(String::from("")),
        },
        IdentityPublicData {
            t: ContactType::Person,
            email: Some(String::from("logistilla@fournier.com")),
            name: String::from("Logistilla Fournier"),
            node_id: keypair.public_key().to_string(),
            postal_address: PostalAddress {
                country: String::from("France"),
                city: String::from("Toulous"),
                zip: Some(String::from("31000")),
                address: String::from("25, rou Pierre de Coubertin"),
            },
            nostr_relay: Some(String::from("")),
        },
        IdentityPublicData {
            t: ContactType::Company,
            email: Some(String::from("moonlimited@ltd.com")),
            name: String::from("Moon Limited"),
            node_id: keypair.public_key().to_string(),
            postal_address: PostalAddress {
                country: String::from("USA"),
                city: String::from("New York"),
                zip: Some(String::from("86659-2593")),
                address: String::from("3443 Joanny Bypass"),
            },
            nostr_relay: Some(String::from("")),
        },
        IdentityPublicData {
            t: ContactType::Company,
            email: Some(String::from("blanco@spa.com")),
            name: String::from("Blanco y Asoc."),
            node_id: keypair.public_key().to_string(),
            postal_address: PostalAddress {
                country: String::from("Argentina"),
                city: String::from("Puerto Clara"),
                zip: Some(String::from("38074")),
                address: String::from("Isidora 96 0 7"),
            },
            nostr_relay: Some(String::from("")),
        },
        IdentityPublicData {
            t: ContactType::Company,
            email: Some(String::from("alexanderurner@grimm.com")),
            name: String::from("Grimm GmbH"),
            node_id: keypair.public_key().to_string(),
            postal_address: PostalAddress {
                country: String::from("Austria"),
                city: String::from("Perg"),
                zip: Some(String::from("3512")),
                address: String::from("Barthring 342"),
            },
            nostr_relay: Some(String::from("")),
        },
        IdentityPublicData {
            t: ContactType::Company,
            email: Some(String::from("antoniosegovia@santiago.com")),
            name: String::from("Empresa Santiago"),
            node_id: keypair.public_key().to_string(),
            postal_address: PostalAddress {
                country: String::from("Spain"),
                city: String::from("Vall Juarez"),
                zip: Some(String::from("88191")),
                address: String::from("Avinguida José Antonio, 20"),
            },
            nostr_relay: Some(String::from("")),
        },
        IdentityPublicData {
            t: ContactType::Company,
            email: Some(String::from("santoro_group@spa.com")),
            name: String::from("Santoro Group"),
            node_id: keypair.public_key().to_string(),
            postal_address: PostalAddress {
                country: String::from("Italy"),
                city: String::from("Prunetta"),
                zip: Some(String::from("51020")),
                address: String::from("Corso Vittorio Emanuele, 90"),
            },
            nostr_relay: Some(String::from("")),
        },
    ];

    let mut rng = rand::thread_rng();
    let random_index = rand::Rng::gen_range(&mut rng, 0..sample.len());
    let random_identity = sample[random_index].clone();
    (keypair, random_identity)
}
