// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{get, post},
    Router,
};
use bcr_common::{
    client::{core::Client as CoreClient, mint::Client as MintClient},
    clwdr_client,
};
use bcr_wdc_utils::surreal;
use bitcoin::bip32 as btc32;
// ----- local modules
mod admin;
pub mod error;
pub mod keys;
pub mod persistence;
pub mod swap;
mod web;

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    keys: surreal::DBConnConfig,
    signatures: surreal::DBConnConfig,
    proofs: surreal::DBConnConfig,
    commitments: surreal::DBConnConfig,
    clowder_url: clwdr_client::Url,
    starting_derivation_path: btc32::DerivationPath,
    max_expiry_sec: u64,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    pub keys: Arc<keys::service::Service>,
    pub swap: Arc<swap::service::Service>,
}

impl AppController {
    pub async fn new(seed: &[u8], cfg: AppConfig) -> Self {
        let AppConfig {
            keys,
            signatures,
            proofs,
            commitments,
            clowder_url,
            starting_derivation_path,
            max_expiry_sec,
        } = cfg;

        let keys_repo = persistence::surreal::DBKeys::new(keys)
            .await
            .expect("DB connection to keys failed");
        let signatures_repo = persistence::surreal::DBSignatures::new(signatures)
            .await
            .expect("DB connection to signatures failed");
        let proofs_repo = persistence::surreal::DBProofs::new(proofs)
            .await
            .expect("Failed to create proofs repository");
        let commitments_repo = persistence::surreal::DBCommitments::new(commitments)
            .await
            .expect("Failed to create commitments repository");
        let keygen = keys::factory::Factory::new(seed, starting_derivation_path);
        let clowder_cl = clwdr_client::ClowderNatsClient::new(clowder_url)
            .await
            .expect("Failed to create clowder client");
        let clowder_cl = Arc::new(clowder_cl);
        let clowder_for_keys = keys::ClowderCl {
            nats: clowder_cl.clone(),
        };
        let keys_service = keys::service::Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            clowder: Box::new(clowder_for_keys),
            keygen,
        };
        let clowder_for_swap = swap::ClowderCl { nats: clowder_cl };
        let max_expiry = chrono::Duration::seconds(max_expiry_sec as i64);
        let swap_service = swap::service::Service {
            proofs: Box::new(proofs_repo),
            commitments: Box::new(commitments_repo),
            clowder: Box::new(clowder_for_swap),
            max_expiry,
        };

        Self {
            keys: Arc::new(keys_service),
            swap: Arc::new(swap_service),
        }
    }
}

pub fn routes<Cntrlr>(ctrl: Cntrlr) -> Router
where
    Cntrlr: Send + Sync + Clone + 'static,
    Arc<keys::service::Service>: FromRef<Cntrlr>,
    Arc<swap::service::Service>: FromRef<Cntrlr>,
{
    let web = Router::new()
        .route("/health", get(get_health))
        .route(MintClient::KEYSETINFO_EP_V1, get(web::lookup_keyset))
        .route(MintClient::LISTKEYSETINFO_EP_V1, get(web::list_keysets))
        .route(MintClient::KEYS_EP_V1, get(web::lookup_keys))
        .route(MintClient::RESTORE_EP_V1, post(web::restore))
        .route(MintClient::SWAP_EP_V1, post(web::swap_tokens))
        .route(MintClient::SWAPCOMMIT_EP_V1, post(web::commit_to_swap))
        .route(MintClient::CHECKSTATE_EP_V1, post(web::check_state));
    // separate admin as it will likely have different auth requirements
    let admin = Router::new()
        .route(CoreClient::NEW_KEYSET_EP_V1, post(admin::new_keyset))
        .route(CoreClient::SIGN_EP_V1, post(admin::sign_blind))
        .route(CoreClient::VERIFY_PROOF_EP_V1, post(admin::verify_proof))
        .route(
            CoreClient::VERIFY_FINGERPRINT_EP_V1,
            post(admin::verify_fingerprint),
        )
        .route(CoreClient::DEACTIVATEKEYSET_EP_V1, post(admin::deactivate))
        .route(CoreClient::BURN_EP_V1, post(admin::burn_tokens))
        .route(CoreClient::RECOVER_EP_V1, post(admin::recover_tokens));

    Router::new().merge(web).merge(admin).with_state(ctrl)
}

async fn get_health() -> &'static str {
    "{ \"status\": \"OK\" }"
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;
    use bcr_wdc_utils::KeysetEntry;
    use std::str::FromStr;

    fn test_controller() -> AppController {
        let seed = [0u8; 32];
        let derivation_path = btc32::DerivationPath::default();
        let keys_repo = persistence::inmemory::KeyMap::default();
        let signatures_repo = persistence::inmemory::SignatureMap::default();
        let keygen = keys::factory::Factory::new(&seed, derivation_path);
        let keysrv = keys::service::Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            keygen,
            clowder: Box::new(keys::DummyClowderClient),
        };
        let proofs_repo = persistence::inmemory::ProofMap::default();
        let commitments_repo = persistence::inmemory::CommitmentMap::default();
        let swprv = swap::service::Service {
            proofs: Box::new(proofs_repo),
            commitments: Box::new(commitments_repo),
            clowder: Box::new(swap::test_utils::DummyClowderClient),
            max_expiry: chrono::Duration::seconds(3600),
        };
        AppController {
            keys: Arc::new(keysrv),
            swap: Arc::new(swprv),
        }
    }

    pub fn mint_kp() -> secp256k1::Keypair {
        let sk = secp256k1::SecretKey::from_str(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .unwrap();
        secp256k1::Keypair::from_secret_key(secp256k1::global::SECP256K1, &sk)
    }

    pub async fn build_test_server(
        keyset: Option<KeysetEntry>,
    ) -> (axum_test::TestServer, AppController) {
        let cfg = axum_test::TestServerConfig {
            transport: Some(axum_test::Transport::HttpRandomPort),
            ..Default::default()
        };
        let cntrl = test_controller();
        if let Some(entry) = keyset {
            cntrl.keys.keys.store(entry).await.expect("store keyset");
        }
        let server = axum_test::TestServer::new_with_config(routes(cntrl.clone()), cfg)
            .expect("failed to start test server");
        (server, cntrl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_common::{
        cashu,
        core::signature::schnorr_verify_b64,
        core_tests,
        wire::{keys as wire_keys, swap as wire_swap},
    };
    use bcr_wdc_utils::{keys::test_utils as keys_test, signatures::test_utils as signatures_test};

    #[tokio::test]
    async fn commit_swap() {
        let (_, controller) = test_utils::build_test_server(None).await;
        let keys_entry = keys_test::generate_keyset();
        controller
            .keys
            .keys
            .store(keys_entry.clone())
            .await
            .expect("store");
        assert!(controller.keys.info(keys_entry.0.id).await.is_ok());
        let amounts = vec![cashu::Amount::from(8_u64)];
        let blinds: Vec<_> = signatures_test::generate_blinds(keys_entry.0.id, &amounts)
            .into_iter()
            .map(|bbb| bbb.0)
            .collect();
        let proofs = core_tests::generate_random_ecash_proofs(&keys_entry.1, &amounts);
        let proof_fps: Vec<wire_keys::ProofFingerprint> = proofs
            .iter()
            .cloned()
            .map(|p| wire_keys::ProofFingerprint::try_from(p))
            .collect::<Result<_, _>>()
            .unwrap();
        let mint_kp = test_utils::mint_kp();
        let now = chrono::Utc::now();
        let expiry = (now + chrono::TimeDelta::minutes(2)).timestamp() as u64;
        let wallet_kp = bitcoin::secp256k1::Keypair::new_global(&mut rand::thread_rng());
        let request = wire_swap::SwapCommitmentRequest {
            inputs: proof_fps,
            outputs: blinds.clone(),
            expiry,
            wallet_key: wallet_kp.public_key().into(),
        };
        let signsrvc = crate::swap::KeysSignService {
            keys: controller.keys.clone(),
        };
        let (content, commitment) = controller
            .swap
            .commit_to_swap(&signsrvc, request, now)
            .await
            .unwrap();
        schnorr_verify_b64(&content, &commitment, &mint_kp.x_only_public_key().0).unwrap();
    }

    #[tokio::test]
    async fn swap() {
        let (_, controller) = test_utils::build_test_server(None).await;
        let keys_entry = keys_test::generate_keyset();
        controller
            .keys
            .keys
            .store(keys_entry.clone())
            .await
            .expect("store");
        assert!(controller.keys.info(keys_entry.0.id).await.is_ok());
        let amounts = vec![cashu::Amount::from(8_u64)];
        let blinds: Vec<_> = signatures_test::generate_blinds(keys_entry.0.id, &amounts)
            .into_iter()
            .map(|bbb| bbb.0)
            .collect();
        let proofs = core_tests::generate_random_ecash_proofs(&keys_entry.1, &amounts);
        let proof_fps: Vec<wire_keys::ProofFingerprint> = proofs
            .iter()
            .cloned()
            .map(|p| wire_keys::ProofFingerprint::try_from(p))
            .collect::<Result<_, _>>()
            .unwrap();
        let mint_kp = test_utils::mint_kp();
        let now = chrono::Utc::now();
        let expiry = (now + chrono::TimeDelta::minutes(2)).timestamp() as u64;
        let wallet_kp = bitcoin::secp256k1::Keypair::new_global(&mut rand::thread_rng());
        let request = wire_swap::SwapCommitmentRequest {
            inputs: proof_fps,
            outputs: blinds.clone(),
            expiry,
            wallet_key: wallet_kp.public_key().into(),
        };
        let signsrvc = crate::swap::KeysSignService {
            keys: controller.keys.clone(),
        };
        let (content, commitment) = controller
            .swap
            .commit_to_swap(&signsrvc, request, now)
            .await
            .unwrap();
        schnorr_verify_b64(&content, &commitment, &mint_kp.x_only_public_key().0).unwrap();

        controller
            .swap
            .swap(&signsrvc, proofs, blinds, commitment, now)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn swap_p2pk() {
        let (_, controller) = test_utils::build_test_server(None).await;
        let keys_entry = keys_test::generate_keyset();
        let kid = keys_entry.0.id;
        controller
            .keys
            .keys
            .store(keys_entry.clone())
            .await
            .expect("store");
        let p2pk_secret = cashu::SecretKey::generate();
        let conditions = cashu::SpendingConditions::new_p2pk(p2pk_secret.public_key(), None);
        let mint_keyset = keys_entry.1;
        let amounts = [cashu::Amount::from(2)];
        let output: Vec<_> = amounts
            .iter()
            .map(|amount| {
                let secret: cashu::nut10::Secret = conditions.clone().into();
                let secret: cashu::secret::Secret = secret.try_into().unwrap();
                let (blinded, r) = cashu::dhke::blind_message(&secret.to_bytes(), None).unwrap();
                let blinded_message = cashu::BlindedMessage::new(*amount, kid, blinded);
                (blinded_message, secret, r)
            })
            .collect();
        let signatures: Vec<_> = output
            .iter()
            .map(|(blinded_message, _, _)| {
                let mint_secret = mint_keyset
                    .keys
                    .get(&blinded_message.amount)
                    .unwrap()
                    .secret_key
                    .clone();
                let c = cashu::dhke::sign_message(&mint_secret, &blinded_message.blinded_secret)
                    .unwrap();
                cashu::nuts::BlindSignature {
                    amount: blinded_message.amount,
                    keyset_id: mint_keyset.id,
                    c,
                    dleq: None,
                }
            })
            .collect();
        let rs = output.iter().map(|(_, _, r)| r.clone()).collect::<Vec<_>>();
        let secrets = output
            .iter()
            .map(|(_, secret, _)| secret.clone())
            .collect::<Vec<_>>();

        let mint_keys = cashu::KeySet::from(mint_keyset.clone()).keys;
        let mut proofs = cashu::dhke::construct_proofs(
            signatures.clone(),
            rs.clone(),
            secrets.clone(),
            &mint_keys,
        )
        .unwrap();
        let blinds: Vec<cashu::BlindedMessage> =
            signatures_test::generate_blinds(mint_keyset.id, &amounts)
                .into_iter()
                .map(|bbb| bbb.0)
                .collect();
        let wallet_kp = bitcoin::secp256k1::Keypair::new_global(&mut rand::thread_rng());
        let now = chrono::Utc::now();
        let expiry = (now + chrono::TimeDelta::minutes(2)).timestamp() as u64;
        let proof_fps: Vec<wire_keys::ProofFingerprint> = proofs
            .iter()
            .cloned()
            .map(|p| wire_keys::ProofFingerprint::try_from(p))
            .collect::<Result<_, _>>()
            .unwrap();
        let request = wire_swap::SwapCommitmentRequest {
            inputs: proof_fps,
            outputs: blinds.clone(),
            expiry,
            wallet_key: wallet_kp.public_key().into(),
        };
        let signsrvc = crate::swap::KeysSignService {
            keys: controller.keys.clone(),
        };
        let (_, commitment) = controller
            .swap
            .commit_to_swap(&signsrvc, request, now)
            .await
            .unwrap();

        let res = controller
            .swap
            .swap(&signsrvc, proofs.clone(), blinds.clone(), commitment, now)
            .await;
        assert!(res.is_err());
        for p in proofs.iter_mut() {
            let _ = p.sign_p2pk(p2pk_secret.clone());
        }
        controller
            .swap
            .swap(&signsrvc, proofs.clone(), blinds, commitment, now)
            .await
            .unwrap();
    }
}
