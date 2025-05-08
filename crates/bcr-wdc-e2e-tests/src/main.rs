// ----- standard library imports

// ----- extra library imports
use bcr_wdc_webapi::keys::ActivateKeysetRequest;
use bcr_wdc_webapi::quotes::{
    BillInfo, EnquireRequest, IdentityPublicData, StatusReply, UpdateQuoteRequest,
    UpdateQuoteResponse,
};
use bdk_wallet::serde_json;
use bitcoin::secp256k1::Keypair;
use cashu::nuts::nut02 as cdk02;
use cashu::{Amount, MintBolt11Request, MintBolt11Response};
use rand::Rng;
use tracing::info;
use tracing_subscriber::filter::LevelFilter;
// ----- local modules

// ----- end imports

#[derive(Debug, serde::Deserialize)]
struct MainConfig {
    wallet_service: String,
    admin_service: String,
    key_service: String,
    quote_service: String,
}

fn setup_tracing() {
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::OFF)
        .init();
}

fn random_ebill() -> (Keypair, BillInfo, bitcoin::secp256k1::schnorr::Signature) {
    let bill_id = bcr_wdc_webapi::test_utils::random_bill_id();
    let (_, drawee) = bcr_wdc_webapi::test_utils::random_identity_public_data();
    let (_, drawer) = bcr_wdc_webapi::test_utils::random_identity_public_data();
    let (_, payee) = bcr_wdc_webapi::test_utils::random_identity_public_data();

    let endorsees_size = rand::thread_rng().gen_range(0..3);
    let mut endorsees: Vec<IdentityPublicData> = Vec::with_capacity(endorsees_size);

    let (endorser_kp, endorser) = bcr_wdc_webapi::test_utils::random_identity_public_data();
    endorsees.push(endorser);

    let owner_key = bcr_wdc_utils::keys::test_utils::generate_random_keypair();

    let amount = Amount::from(rand::thread_rng().gen_range(1000..100000));

    let bill = BillInfo {
        id: bill_id,
        maturity_date: random_date(),
        drawee,
        drawer,
        payee,
        endorsees,
        sum: amount.into(),
    };

    let signature = bcr_wdc_utils::keys::schnorr_sign_borsh_msg_with_key(&bill, &endorser_kp)
        .expect("schnorr_sign_borsh_msg_with_key");

    (owner_key, bill, signature)
}

fn random_date() -> String {
    let start = chrono::Utc::now() + chrono::Duration::days(365);
    let days = rand::thread_rng().gen_range(0..365);
    (start + chrono::Duration::days(days)).to_rfc3339()
}

fn get_amounts(mut targ: u64) -> Vec<u64> {
    // TODO see if there is an existing cashu implementation
    let mut coins = Vec::new();
    let mut bit_position = 0;
    while targ > 0 {
        if (targ & 1) == 1 {
            coins.push(1 << bit_position);
        }
        targ >>= 1;
        bit_position += 1;
    }
    coins
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

async fn can_mint_ebill(cfg: &MainConfig) {
    setup_tracing();

    let wallet_service_url = cfg.wallet_service.clone();
    let admin_service_url = cfg.admin_service.clone();
    let key_service_url = cfg.key_service.clone();
    let quote_service_url = cfg.quote_service.clone();

    let (owner_key, bill, signature) = random_ebill();

    let request = EnquireRequest {
        content: bill,
        public_key: owner_key.public_key().into(),
        signature,
    };

    let request_json = serde_json::to_string(&request).expect("Failed to serialize request");
    let bill_amount = request.content.sum;

    info!(
        bill_amount = bill_amount,
        bill_id = request.content.id,
        "Bill created"
    );

    let client = reqwest::Client::new();

    info!("Requesting to mint the bill");
    let mint_credit_quote_url = format!("{}/v1/mint/credit/quote", wallet_service_url);
    let response = client
        .post(mint_credit_quote_url)
        .header("Content-Type", "application/json")
        .body(request_json)
        .send()
        .await
        .unwrap();

    let enquire_reply = response
        .json::<bcr_wdc_webapi::quotes::EnquireReply>()
        .await
        .unwrap();
    let quote_id = enquire_reply.id;

    info!(quote_id = ?quote_id, "Mint Request Accepted, waiting for admin to process");

    let one_year_from_now = chrono::Utc::now() + chrono::Duration::days(365);
    let discounted_offer = bill_amount * 99 / 100;
    let update_quote_request_payload = UpdateQuoteRequest::Offer {
        discounted: bitcoin::Amount::from_sat(discounted_offer),
        ttl: Some(one_year_from_now),
    };

    let update_quote_request_json = serde_json::to_string(&update_quote_request_payload)
        .expect("Failed to serialize UpdateQuoteRequest");

    let admin_update_quote_url =
        format!("{}/v1/admin/credit/quote/{}", admin_service_url, quote_id);
    info!(
        discounted = discounted_offer,
        "Admin sending discounted offer"
    );
    let update_response = client
        .post(&admin_update_quote_url)
        .header("Content-Type", "application/json")
        .body(update_quote_request_json)
        .send()
        .await
        .unwrap();

    let update_quote_response_body = update_response.json::<UpdateQuoteResponse>().await.unwrap();
    match update_quote_response_body {
        UpdateQuoteResponse::Denied => {
            info!("Quote is denied");
        }
        UpdateQuoteResponse::Offered { discounted, ttl } => {
            info!(amount=%discounted, ttl=%ttl, "Quote is offered")
        }
    }

    let mint_quote_status_url = format!("{}/v1/mint/credit/quote/{}", quote_service_url, quote_id);
    info!("Getting mint quote status from: {}", mint_quote_status_url);
    let mint_status_response = client.get(&mint_quote_status_url).send().await.unwrap();

    let mint_quote_status_reply = mint_status_response.json::<StatusReply>().await.unwrap();

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

    let activate_request_payload = ActivateKeysetRequest { qid: quote_id };
    let activate_request_json = serde_json::to_string(&activate_request_payload)
        .expect("Failed to serialize ActivateKeysetRequest");

    info!("Activating keyset for quote_id: {}", quote_id);
    let activate_url = format!("{}/v1/admin/keys/activate", admin_service_url);
    let activate_response = client
        .post(activate_url)
        .header("Content-Type", "application/json")
        .body(activate_request_json)
        .send()
        .await
        .unwrap();

    info!(
        "Activate keyset response status: {:?}",
        activate_response.status()
    );

    let keysets_url = format!("{}/v1/keysets", wallet_service_url);
    let keysets_response = client.get(keysets_url).send().await.unwrap();
    let keysets = keysets_response
        .json::<cdk02::KeysetResponse>()
        .await
        .unwrap();
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
    let mint_url = format!("{}/v1/mint/ebill", key_service_url);
    let mint_response = client
        .post(mint_url)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&req).unwrap())
        .send()
        .await
        .unwrap();

    let mint_response_body = mint_response.json::<MintBolt11Response>().await.unwrap();
    info!("Mint response: {:?}", mint_response_body);

    let blinded_signatures = mint_response_body.signatures;

    let total_amount = blinded_signatures
        .iter()
        .map(|s| u64::from(s.amount))
        .sum::<u64>();
    assert_eq!(total_amount, bill_amount);
}

fn main() {
    let settings = config::Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("WALLET_AGGREGATOR"))
        .build()
        .expect("Failed to build wildcat config");

    let cfg: MainConfig = settings
        .try_deserialize()
        .expect("Failed to parse wildcat config");

    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(can_mint_ebill(&cfg));
}
