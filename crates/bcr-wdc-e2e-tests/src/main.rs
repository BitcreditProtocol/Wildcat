// ----- standard library imports
// ----- extra library imports
use bcr_wdc_webapi::keys::ActivateKeysetRequest;
use bcr_wdc_webapi::quotes::EnquireReply;
use bcr_wdc_webapi::quotes::{
    EnquireRequest, StatusReply, UpdateQuoteRequest, UpdateQuoteResponse,
};

use cashu::nuts::nut02 as cdk02;
use cashu::{MintBolt11Request, MintBolt11Response};
use tracing::info;
use tracing_subscriber::filter::LevelFilter;
// ----- local modules
mod endpoints;
mod rest_client;
mod test_utils;
use endpoints::*;
use rest_client::RestClient;
use test_utils::{generate_blinds, get_amounts, random_ebill};
// ----- end imports

#[derive(Debug, serde::Deserialize)]
struct MainConfig {
    wallet_service: String,
    admin_service: String,
    key_service: String,
}

fn setup_tracing() {
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::INFO)
        .init();
}

async fn can_mint_ebill(cfg: &MainConfig) {
    setup_tracing();

    info!("Starting ebill minting test");

    let api = RestClient::new();

    let wallet_service = Endpoints::<WalletService>::new(cfg.wallet_service.clone());
    let admin_service = Endpoints::<AdminService>::new(cfg.admin_service.clone());
    let keys_service = Endpoints::<KeyService>::new(cfg.key_service.clone());

    // Create Ebill
    let (owner_key, bill, signature) = random_ebill();

    let request = EnquireRequest {
        content: bill,
        public_key: owner_key.public_key().into(),
        signature,
    };

    let bill_amount = request.content.sum;

    info!(
        bill_amount = bill_amount,
        bill_id = request.content.id,
        "Bill created"
    );

    // Mint Ebill
    info!("Requesting to mint the bill");
    let mint_credit_quote_url = wallet_service.mint_credit_quote_url();
    let enquire_reply: EnquireReply = api.post(mint_credit_quote_url, &request).await.unwrap();
    let quote_id = enquire_reply.id;

    info!(quote_id = ?quote_id, "Mint Request Accepted, waiting for admin to process");

    let one_year_from_now = chrono::Utc::now() + chrono::Duration::days(365);
    let discounted_offer = bill_amount * 99 / 100;
    let update_quote_request_payload = UpdateQuoteRequest::Offer {
        discounted: bitcoin::Amount::from_sat(discounted_offer),
        ttl: Some(one_year_from_now),
    };

    info!(
        discounted = discounted_offer,
        "Admin sending discounted offer"
    );

    let update_quote_response: UpdateQuoteResponse = api
        .post(
            admin_service.admin_credit_quote(&quote_id.to_string()),
            &update_quote_request_payload,
        )
        .await
        .unwrap();

    match update_quote_response {
        UpdateQuoteResponse::Denied => {
            info!("Quote is denied");
        }
        UpdateQuoteResponse::Offered { discounted, ttl } => {
            info!(amount=%discounted, ttl=%ttl, "Quote is offered")
        }
    }

    let mint_quote_status_url = wallet_service.lookup_credit_quote(&quote_id.to_string());
    info!("Getting mint quote status from: {}", mint_quote_status_url);
    let mint_quote_status_reply: StatusReply = api.get(mint_quote_status_url).await.unwrap();

    if let StatusReply::Accepted { keyset_id } = mint_quote_status_reply {
        info!(keyset_id=%keyset_id, "Quote is accepted");
    } else {
        panic!("Quote is not accepted");
    }

    let keyset_id = match mint_quote_status_reply {
        StatusReply::Offered { keyset_id, .. } => Some(keyset_id),
        StatusReply::Accepted { keyset_id, .. } => Some(keyset_id),
        _ => None,
    }
    .unwrap();

    // Activate keyset
    let activate_request_payload = ActivateKeysetRequest { qid: quote_id };
    info!("Activating keyset for quote_id: {}", quote_id);
    api.post_(admin_service.keys_activate(), &activate_request_payload)
        .await
        .unwrap();

    let keysets: cdk02::KeysetResponse = api.get(wallet_service.list_keysets()).await.unwrap();
    assert!(keysets.keysets.iter().any(|ks| ks.id == keyset_id));
    assert!(keysets.keysets.iter().any(|ks| ks.active));
    let keyset_info = keysets
        .keysets
        .iter()
        .find(|ks| ks.id == keyset_id)
        .unwrap();
    assert!(keyset_info.active);

    info!(keyset_info_id = ?keyset_info.id, "Confirmed active keyset");

    let amounts = get_amounts(bill_amount)
        .iter()
        .map(|a| cashu::Amount::from(*a))
        .collect::<Vec<_>>();
    let blinds = generate_blinds(keyset_info.id, &amounts);
    let blinded_messages = blinds.iter().map(|b| b.0.clone()).collect::<Vec<_>>();

    info!("Signing NUT20 mint request");
    let mut req = MintBolt11Request {
        quote: quote_id,
        outputs: blinded_messages,
        signature: None,
    };
    req.sign(owner_key.secret_key().into()).unwrap();

    info!("Sending NUT20 mint request");
    let mint_response: MintBolt11Response =
        api.post(keys_service.mint_ebill(), &req).await.unwrap();
    let blinded_signatures = mint_response.signatures;

    let total_amount = blinded_signatures
        .iter()
        .map(|s| u64::from(s.amount))
        .sum::<u64>();
    assert_eq!(total_amount, bill_amount);
    info!(amount = total_amount, "Mint Successful obtained signatures");
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

    can_mint_ebill(&cfg).await;
}
