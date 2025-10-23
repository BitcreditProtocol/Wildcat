// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use bcr_common::wire::quotes::{StatusReply, UpdateQuoteResponse};
use cashu::MintUrl;

use reqwest::Url;
use tracing::info;
use tracing_subscriber::filter::LevelFilter;
// ----- local modules
mod clients;
mod test_utils;
use clients::*;
use test_utils::{generate_blinds, random_ebill_request, EbillRequestComponents};
// ----- end imports

#[derive(Debug, serde::Deserialize)]
struct MainConfig {
    user_service: String,
    admin_service: String,
    keycloak: KeycloakConfig,
}

#[derive(Debug, serde::Deserialize)]
struct KeycloakConfig {
    url: Url,
    client_id: String,
    client_secret: String,
    username: String,
    password: String,
}

fn setup_tracing() {
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::INFO)
        .init();
}

async fn test_auth(cfg: &MainConfig) {
    info!("Auth Test");

    let mut admin_service = Service::<AdminService>::new(cfg.admin_service.clone());
    admin_service
        .authenticate(
            cfg.keycloak.url.clone(),
            &cfg.keycloak.client_id,
            &cfg.keycloak.client_secret,
            &cfg.keycloak.username,
            &cfg.keycloak.password,
        )
        .await
        .unwrap();
    let _ = admin_service.admin_credit_quote_list().await;

    info!("Testing admin service without authorization");
    let admin_service = Service::<AdminService>::new(cfg.admin_service.clone());
    let resp = admin_service.admin_credit_quote_list().await;
    assert!(resp.is_err());

    info!("Testing admin service with wrong credentials");
    let mut admin_service = Service::<AdminService>::new(cfg.admin_service.clone());
    if admin_service
        .authenticate(
            cfg.keycloak.url.clone(),
            &cfg.keycloak.client_id,
            &cfg.keycloak.client_secret,
            "wrong_username",
            "wrong_password",
        )
        .await
        .is_ok()
    {
        panic!("Got token using wrong credentials");
    }

    // Test auth on admin_balance_credit
    let admin_service = Service::<AdminService>::new(cfg.admin_service.clone());
    let res = admin_service.admin_balance_credit().await;
    assert!(res.is_err());
    info!("Testing admin_balance_credit with authorization");
    let mut admin_service = Service::<AdminService>::new(cfg.admin_service.clone());
    admin_service
        .authenticate(
            cfg.keycloak.url.clone(),
            &cfg.keycloak.client_id,
            &cfg.keycloak.client_secret,
            &cfg.keycloak.username,
            &cfg.keycloak.password,
        )
        .await
        .unwrap();
    let balance = admin_service.admin_balance_credit().await.unwrap();
    info!(balance = ?balance.amount, unit = ?balance.unit, "Admin balance");

    // Test protected endpoints
    let urls = vec![
        // generic
        "v1/admin/credit",
        "v1/admin/balance/",
        "v1/admin/keys",
        "v1/admin/onchain",
        // specific
        "v1/admin/credit/quote/enable_mint/0000",
        "v1/admin/credit/quote/0000",
    ];
    let http = reqwest::Client::builder().build().unwrap();
    for url in urls {
        info!(url=?url, "Testing if authorization is required");
        let url = Url::parse(&format!("{}/{}", cfg.admin_service, url)).unwrap();
        // GET
        let response = http.get(url.clone()).send().await.unwrap();
        assert_eq!(response.status(), 401);
        // POST
        let response = http.post(url).send().await.unwrap();
        assert_eq!(response.status(), 401);
    }
}

async fn can_mint_ebill(cfg: &MainConfig) {
    info!("Ebill minting test");

    let user_service = Service::<UserService>::new(cfg.user_service.clone());
    let mut admin_service = Service::<AdminService>::new(cfg.admin_service.clone());
    info!("Authenticating admin service");
    admin_service
        .authenticate(
            cfg.keycloak.url.clone(),
            &cfg.keycloak.client_id,
            &cfg.keycloak.client_secret,
            &cfg.keycloak.username,
            &cfg.keycloak.password,
        )
        .await
        .unwrap();
    info!("Admin service authenticated");

    let mint_info = user_service.mint_info().await;
    let mint_name = mint_info.name.unwrap();
    let mint_description = mint_info.description.unwrap();
    info!(name = mint_name, desc = mint_description, "Mint info");

    let identity = admin_service.admin_ebill_identity_details().await.unwrap();
    let bill_amount = bitcoin::Amount::from_btc(0.001).unwrap();
    // Create Ebill
    let EbillRequestComponents {
        bill, signing_key, ..
    } = random_ebill_request(identity.bitcoin_public_key, Some(bill_amount));

    info!(
        bill_amount = bill_amount.to_sat(),
        bill_id = bill.bill_id.to_string(),
        "Bill created"
    );

    // Mint Ebill
    info!("Requesting to mint the bill");
    // let quote_id = user_service.mint_credit_quote(bill, owner_key.public_key().into(), &signing_key).await;
    let quote_id = user_service
        .mint_credit_quote(bill, signing_key.public_key().into(), &signing_key)
        .await;

    info!(quote_id = ?quote_id, "Mint Request Accepted, waiting for admin to process");

    let admin_discounted_offer = bill_amount * 99 / 100;

    info!(
        discounted = admin_discounted_offer.to_sat(),
        "Admin sending discounted offer"
    );

    let update_quote_response: UpdateQuoteResponse = admin_service
        .offer_quote(quote_id, admin_discounted_offer)
        .await;

    match update_quote_response {
        UpdateQuoteResponse::Denied => {
            info!("Quote is denied");
        }
        UpdateQuoteResponse::Offered { discounted, ttl } => {
            info!(amount=%discounted, ttl=%ttl, "Quote is offered")
        }
    }

    let mint_quote_status_reply = user_service.lookup_credit_quote(quote_id).await;
    info!(quote_id=?quote_id, "Getting mint quote status for quote");

    let offered_discount;
    if let StatusReply::Offered {
        keyset_id,
        expiration_date,
        discounted,
        ..
    } = mint_quote_status_reply
    {
        info!(keyset_id=?keyset_id, expiration_date=?expiration_date, "Quote is offered");
        offered_discount = discounted;
    } else {
        panic!("Quote is not accepted");
    }

    let keyset_id = match mint_quote_status_reply {
        StatusReply::Offered { keyset_id, .. } => Some(keyset_id),
        StatusReply::Accepted { keyset_id, .. } => Some(keyset_id),
        _ => None,
    }
    .unwrap();

    user_service.accept_quote(quote_id).await;
    admin_service.enable_minting_for_quote_id(quote_id).await;

    let keysets = user_service.list_keysets().await;
    assert!(keysets.iter().any(|ks| ks.id == keyset_id));
    assert!(keysets.iter().any(|ks| ks.active));
    let keyset_info = keysets.iter().find(|ks| ks.id == keyset_id).unwrap();
    assert!(keyset_info.active);

    info!(keyset_info_id = ?keyset_info.id, "Confirmed active keyset");

    let cashu_amounts = cashu::Amount::from(offered_discount.to_sat()).split();
    let blinds = generate_blinds(keyset_info.id, &cashu_amounts);
    let blinded_messages = blinds.iter().map(|b| b.0.clone()).collect::<Vec<_>>();

    info!("Sending NUT20 mint request");
    let blinded_signatures = user_service
        .mint_ebill(quote_id, blinded_messages, signing_key.secret_key().into())
        .await;

    let total_amount = blinded_signatures
        .iter()
        .map(|s| u64::from(s.amount))
        .sum::<u64>();
    assert_eq!(total_amount, offered_discount.to_sat());
    info!(amount = total_amount, "Mint Successful obtained signatures");

    // Needed to unblind the signatures
    let keys = user_service.list_keys(keyset_id).await;

    let secrets = blinds.iter().map(|b| b.1.clone()).collect::<Vec<_>>();
    let rs = blinds.iter().map(|b| b.2.clone()).collect::<Vec<_>>();
    // blinded messages has B, x - some Secret, r - blindingFactor
    // C_ = C - rK, with C being the signature, r the blinding factor and K with public key of the mint (keyset pubkey for amount)
    let proofs =
        cashu::dhke::construct_proofs(blinded_signatures, rs, secrets, &keys.keys).unwrap();
    info!("Got credit tokens");

    for p in &proofs {
        info!(amount=?p.amount, unblinded=?p.c, secret = ?p.secret, "Proof");
    }

    // Test Swap
    info!("Swapping proofs");
    let new_blinds = generate_blinds(keyset_info.id, &cashu_amounts);
    let bs = new_blinds.iter().map(|b| b.0.clone()).collect::<Vec<_>>();
    let signatures = user_service.swap(proofs, bs).await;
    let total_swap = signatures.iter().map(|s| u64::from(s.amount)).sum::<u64>();
    assert_eq!(
        total_swap, total_amount,
        "Total swap amount does not match total amount"
    );
    let secrets = new_blinds.iter().map(|b| b.1.clone()).collect::<Vec<_>>();
    let rs = new_blinds.iter().map(|b| b.2.clone()).collect::<Vec<_>>();
    let proofs = cashu::dhke::construct_proofs(signatures, rs, secrets, &keys.keys).unwrap();

    let url = &cfg.user_service;
    let mint_url = MintUrl::from_str(url).unwrap();
    let token = cashu::nut00::Token::new(
        mint_url,
        proofs,
        None,
        cashu::CurrencyUnit::Custom("crsat".into()),
    );
    info!(token = token.to_v3_string(), "Swapped Crsat");
}

#[tokio::main]
async fn main() {
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("E2E_TESTS"))
        .build()
        .expect("Failed to build wildcat config");

    let cfg: MainConfig = settings
        .try_deserialize()
        .expect("Failed to parse configuration");

    setup_tracing();

    can_mint_ebill(&cfg).await;
    test_auth(&cfg).await;
}
