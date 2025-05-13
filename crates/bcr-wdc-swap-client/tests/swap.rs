// ----- standard library imports
// ----- extra library imports
use bcr_wdc_key_service::MintCondition;
use bcr_wdc_swap_client::SwapClient;
use bcr_wdc_utils::{keys::test_utils as keys_test, signatures::test_utils as signatures_test};
use cashu::{
    dhke::{blind_message, construct_proofs, sign_message},
    Amount,
};
// ----- local imports

#[tokio::test]
async fn swap() {
    let (server, keys_service) = bcr_wdc_swap_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = SwapClient::new(server_url);

    let keys_entry = keys_test::generate_keyset();
    let condition = MintCondition {
        target: Amount::ZERO,
        pub_key: keys_test::publics()[0],
        is_minted: true,
    };
    keys_service
        .keys
        .keys
        .store(keys_entry.clone(), condition)
        .expect("store");

    let amounts = vec![Amount::from(8_u64)];
    let blinds = signatures_test::generate_blinds(keys_entry.1.id, &amounts)
        .into_iter()
        .map(|bbb| bbb.0)
        .collect();
    let proofs = signatures_test::generate_proofs(&keys_entry.1, &amounts);

    client.swap(proofs, blinds).await.expect("swap");
}

#[tokio::test]
async fn swap_p2pk() {
    let (server, keys_service) = bcr_wdc_swap_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = SwapClient::new(server_url);

    let keys_entry = keys_test::generate_keyset();
    let kid = keys_entry.0.id;

    let condition = MintCondition {
        target: Amount::ZERO,
        pub_key: keys_test::publics()[0],
        is_minted: true,
    };

    keys_service
        .keys
        .keys
        .store(keys_entry.clone(), condition)
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

    let mut incorrect_proofs: Vec<cashu::Proof> =
        construct_proofs(signatures.clone(), rs.clone(), secrets.clone(), &mint_keys).unwrap();
    for p in incorrect_proofs.iter_mut() {
        let _ = p.sign_p2pk(cashu::SecretKey::generate());
    }

    let missing_p2pk_sig_proofs: Vec<cashu::Proof> =
        cashu::dhke::construct_proofs(signatures, rs, secrets, &mint_keys).unwrap();

    // Swap 2,2,4 proofs into a single 8 blinded message
    let single_amount = [Amount::from(8)];
    let blinds: Vec<cashu::BlindedMessage> =
        signatures_test::generate_blinds(mint_keyset.id, &single_amount)
            .into_iter()
            .map(|bbb| bbb.0)
            .collect();

    let res = client
        .swap(correct_proofs, blinds.clone())
        .await
        .expect("Swap with correct P2PK signatures should succeed");
    assert_eq!(res[0].amount, Amount::from(8));
    client
        .swap(incorrect_proofs, blinds.clone())
        .await
        .expect_err("Swap with incorrect P2PK signatures should fail");
    client
        .swap(missing_p2pk_sig_proofs, blinds)
        .await
        .expect_err("Swap with missing P2PK signatures should fail");
}
