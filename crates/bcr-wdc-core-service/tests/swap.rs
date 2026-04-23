// ----- standard library imports
// ----- extra library imports
use bcr_common::{
    cashu::{
        self,
        dhke::{blind_message, construct_proofs, sign_message},
        Amount,
    },
    client::admin::core::Client as CoreClient,
    core_tests,
    wire::keys as wire_keys,
};
use bcr_wdc_utils::{keys::test_utils as keys_test, signatures::test_utils as signatures_test};
// ----- local imports

// ----- end imports

#[tokio::test]
async fn swap() {
    let (server, controller) = bcr_wdc_core_service::test_utils::build_test_server(None).await;
    let server_url = server.server_address().expect("address");
    let client = CoreClient::new(server_url);
    let keys_entry = keys_test::generate_keyset();
    controller
        .keys
        .keys
        .store(keys_entry.clone())
        .await
        .expect("store");
    let amounts = vec![Amount::from(8_u64)];
    let blinds: Vec<_> = signatures_test::generate_blinds(keys_entry.1.id, &amounts)
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
    let mint_kp = bcr_wdc_core_service::test_utils::mint_kp();
    let mint_pk = mint_kp.public_key();
    let expiry = (chrono::Utc::now() + chrono::TimeDelta::minutes(2)).timestamp() as u64;
    let wallet_kp = bitcoin::secp256k1::Keypair::new_global(&mut rand::thread_rng());
    let commitment = client
        .commit_swap(
            proof_fps,
            blinds.clone(),
            expiry,
            wallet_kp.public_key(),
            mint_pk,
        )
        .await
        .unwrap();
    client.swap(proofs, blinds, commitment).await.expect("swap");
}

#[tokio::test]
async fn swap_p2pk() {
    let (server, controller) = bcr_wdc_core_service::test_utils::build_test_server(None).await;
    let server_url = server.server_address().expect("address");
    let client = CoreClient::new(server_url);
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
    let amounts = [Amount::from(2), Amount::from(2), Amount::from(4)];
    let output: Vec<_> = amounts
        .iter()
        .map(|amount| {
            let secret: cashu::nut10::Secret = conditions.clone().into();
            let secret: cashu::secret::Secret = secret.try_into().unwrap();
            let (blinded, r) = blind_message(&secret.to_bytes(), None).unwrap();
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
            let c = sign_message(&mint_secret, &blinded_message.blinded_secret).unwrap();
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
    let mut correct_proofs =
        construct_proofs(signatures.clone(), rs.clone(), secrets.clone(), &mint_keys).unwrap();
    for p in correct_proofs.iter_mut() {
        let _ = p.sign_p2pk(p2pk_secret.clone());
    }
    // Swap 2,2,4 proofs into a single 8 blinded message
    let single_amount = [Amount::from(8)];
    let blinds: Vec<cashu::BlindedMessage> =
        signatures_test::generate_blinds(mint_keyset.id, &single_amount)
            .into_iter()
            .map(|bbb| bbb.0)
            .collect();
    let correct_fps: Vec<wire_keys::ProofFingerprint> = correct_proofs
        .iter()
        .cloned()
        .map(|p| wire_keys::ProofFingerprint::try_from(p))
        .collect::<Result<_, _>>()
        .unwrap();
    for (p, fps) in correct_proofs.iter().zip(correct_fps.iter()) {
        assert_eq!(p.y().unwrap(), fps.y);
    }
    let mint_kp = bcr_wdc_core_service::test_utils::mint_kp();
    let mint_pk = mint_kp.public_key();
    let wallet_kp = bitcoin::secp256k1::Keypair::new_global(&mut rand::thread_rng());
    let expiry = (chrono::Utc::now() + chrono::TimeDelta::minutes(2)).timestamp() as u64;
    let commitment = client
        .commit_swap(
            correct_fps,
            blinds.clone(),
            expiry,
            wallet_kp.public_key(),
            mint_pk,
        )
        .await
        .unwrap();
    let res = client
        .swap(correct_proofs, blinds, commitment)
        .await
        .expect("Swap with correct P2PK signatures should succeed");
    assert_eq!(res[0].amount, Amount::from(8));
}
