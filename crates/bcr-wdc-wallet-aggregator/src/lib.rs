// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{get, post},
    Router,
};
use bcr_common::client::{
    admin::{clowder, core},
    Url as ClientUrl,
};
// ----- local modules
mod error;
mod web;
// ----- local imports
use error::Result;

// ----- end imports

pub use bcr_common::TStamp;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    core_client_url: bcr_common::client::Url,
    treasury_client_url: bcr_common::client::Url,
    clwdr_rest_url: ClientUrl,
    ebill_client_url: ClientUrl,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    core_client: bcr_common::client::core::Client,
    treasury_client: bcr_common::client::treasury::Client,
    clwdr_rest_client: Arc<clowder::Client>,
    time_started: time::OffsetDateTime,
    ebill_client: bcr_common::client::ebill::Client,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig {
            core_client_url,
            treasury_client_url,
            clwdr_rest_url,
            ebill_client_url,
        } = cfg;

        let core_client = bcr_common::client::core::Client::new(core_client_url);
        let treasury_client = bcr_common::client::treasury::Client::new(treasury_client_url);
        let clwdr_rest_client = Arc::new(clowder::Client::new(clwdr_rest_url));

        Self {
            core_client,
            treasury_client,
            clwdr_rest_client,
            time_started: time::OffsetDateTime::now_utc(),
            ebill_client: bcr_common::client::ebill::Client::new(ebill_client_url),
        }
    }
}

pub async fn routes(app: AppController) -> Result<Router> {
    let router = Router::new()
        .route("/health", get(web::health))
        .route("/v1/info", get(web::get_mint_info))
        .route(
            bcr_common::client::ebill::Client::GET_MINT_IDENTITY_EP_V1,
            get(web::get_mint_identity),
        )
        .route(core::web_ep::SWAP_V1, post(web::post_swap))
        .with_state(app);
    Ok(router)
}

#[cfg(test)]
mod mint_identity_tests {
    use super::*;
    use bcr_common::{
        core::NodeId,
        wire::mint_identity::{MintIdentity, MINT_IDENTITY_PATH},
    };
    use bitcoin::{
        secp256k1::{Keypair, SecretKey, SECP256K1},
        Network,
    };

    #[tokio::test]
    async fn wallet_route_forwards_signed_proof_and_rejects_invalid_challenges() {
        let mut signer = mockito::Server::new_async().await;
        let keys = Keypair::from_secret_key(SECP256K1, &SecretKey::from_slice(&[2; 32]).unwrap());
        let node = NodeId::new(keys.public_key(), Network::Regtest);
        let challenge = "cd".repeat(32);
        let proof = MintIdentity::sign(node.clone(), challenge.clone(), 1700000000, &keys).unwrap();
        let endpoint = signer
            .mock("GET", MINT_IDENTITY_PATH)
            .match_query(mockito::Matcher::UrlEncoded(
                "challenge".to_owned(),
                challenge.clone(),
            ))
            .with_header("content-type", "application/json")
            .with_body(serde_json::to_vec(&proof).unwrap())
            .expect(1)
            .create_async()
            .await;
        let unused = "http://127.0.0.1:1".parse::<ClientUrl>().unwrap();
        let ctrl = AppController::new(AppConfig {
            core_client_url: unused.clone(),
            treasury_client_url: unused.clone(),
            clwdr_rest_url: unused,
            ebill_client_url: signer.url().parse().unwrap(),
        })
        .await;
        let server = axum_test::TestServer::new(routes(ctrl).await.unwrap()).unwrap();
        let response = server
            .get(MINT_IDENTITY_PATH)
            .add_query_param("challenge", &challenge)
            .await;
        response.assert_status_ok();
        response.assert_header("cache-control", "no-store");
        assert!(response
            .json::<MintIdentity>()
            .verify(&node, &challenge, 1700000000));
        for bad in [
            "".to_owned(),
            "a".repeat(63),
            "a".repeat(65),
            "A".repeat(64),
            "g".repeat(64),
        ] {
            server
                .get(MINT_IDENTITY_PATH)
                .add_query_param("challenge", bad)
                .await
                .assert_status_bad_request();
        }
        server
            .get(MINT_IDENTITY_PATH)
            .await
            .assert_status_bad_request();
        server
            .get(MINT_IDENTITY_PATH)
            .add_query_param("challenge", &challenge)
            .add_query_param("unknown", "1")
            .await
            .assert_status_bad_request();
        server
            .get("/v1/admin/identity/detail")
            .await
            .assert_status_not_found();
        server
            .get("/v1/admin/identity/seed")
            .await
            .assert_status_not_found();
        endpoint.assert_async().await;
    }
}
