// ----- standard library imports
// ----- extra library imports
use bcr_wdc_key_service::MintCondition;
use bcr_wdc_swap_client::SwapClient;
use bcr_wdc_utils::{keys::test_utils as keys_test, signatures::test_utils as signatures_test};
use cashu::Amount;
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
    let blinds = signatures_test::generate_blinds(&keys_entry.1, &amounts)
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

    keys_service
        .keys
        .keys
        .store(keys_entry.clone())
        .expect("store");


    let p2pk_secret = cashu::SecretKey::generate();
    let conditions = cashu::SpendingConditions::new_p2pk(p2pk_secret.public_key(), None);
    
    let mint_keyset = keys_entry.1;
    let mut output = Vec::with_capacity(3);

    let amounts = [1,2,4].iter().map(|&x| Amount::from(x)).collect::<Vec<_>>();
    for amount in &amounts {
        let secret: cashu::nut10::Secret = conditions.clone().into();

        let secret: cashu::secret::Secret = secret.try_into().unwrap();
        let (blinded, r) = cashu::dhke::blind_message(&secret.to_bytes(), None).unwrap();

        let blinded_message = cashu::BlindedMessage::new(*amount, kid, blinded);

        output.push( (blinded_message, secret, r) )
    }

    let mut signatures = Vec::new();
    let mut mint_pubkeys = Vec::new();
    for (i,amount) in amounts.iter().enumerate() {


        let (blinded_message, blinded_secret, _) = output.get(i).unwrap();
        let mint_secret = mint_keyset.keys.get(&blinded_message.amount).unwrap().secret_key.clone();
        mint_pubkeys.push(mint_keyset.keys.get(&blinded_message.amount).unwrap().public_key);
        let c = cashu::dhke::sign_message(&mint_secret, &blinded_message.blinded_secret).unwrap();  

        assert_eq!(*amount,blinded_message.amount);

        let blinded_signature = cashu::nuts::BlindSignature {  
            amount: blinded_message.amount,  
            keyset_id: mint_keyset.id,  
            c,  
            dleq: None // You can add DLEQ proof if needed  
        };
        signatures.push(blinded_signature);
    }


    let rs = output.iter().map(|(_, _, r)| r.clone()).collect::<Vec<_>>();
    let secrets = output.iter().map(|(_, secret, _)| secret.clone()).collect::<Vec<_>>();
    let collected_keys: std::collections::BTreeMap<cashu::Amount, cashu::PublicKey> = mint_keyset.keys.iter().map(|(amount, mint_key_pair)| (*amount, mint_key_pair.public_key)).collect();
    let cashu_keys = cashu::Keys::new(collected_keys); 
    let mut proofs = cashu::dhke::construct_proofs(  signatures,  rs,  secrets,   &cashu_keys ).unwrap();  
    for p in proofs.iter_mut() {
        let _ = p.sign_p2pk(p2pk_secret.clone());
    }

    println!("proofs: {:?}", proofs);
    println!("working proofs: {:?}", signatures_test::generate_proofs(&mint_keyset, &amounts));

    let blinds = signatures_test::generate_blinds(&mint_keyset, &amounts).into_iter()
        .map(|bbb| bbb.0)
        .collect();

    for p in proofs.iter() {
        bcr_wdc_utils::keys::verify_with_keys(&mint_keyset, p).unwrap();
    }

    client.swap(proofs, blinds).await.expect("swap");
}