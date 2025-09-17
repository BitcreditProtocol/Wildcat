// ----- standard library imports
// ----- extra library imports
use bcr_wdc_key_service::MintOperation;
use bcr_wdc_swap_client::SwapClient;
use bcr_wdc_utils::{keys::test_utils as keys_test, signatures::test_utils as signatures_test};
use cashu::{dhke::hash_to_curve, nut07 as cdk07, Amount};
// ----- local imports

#[tokio::test]
async fn checkstate() {
    let (server, keys_service) = bcr_wdc_swap_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = SwapClient::new(server_url);

    let mut keys_entry = keys_test::generate_keyset();
    let condition = MintOperation {
        target: Amount::ZERO,
        pub_key: keys_test::publics()[0],
        is_minted: true,
    };
    keys_entry.0.active = false;
    keys_service
        .keys
        .keys
        .store(keys_entry.clone(), condition)
        .expect("store");

    let amounts = vec![Amount::from(8_u64), Amount::from(16_u64)];
    let spent = signatures_test::generate_proofs(&keys_entry.1, &amounts);

    client.burn(spent.clone()).await.expect("burn");

    let amounts = vec![Amount::from(32_u64), Amount::from(64_u64)];
    let unspent = signatures_test::generate_proofs(&keys_entry.1, &amounts);

    let ys = vec![
        hash_to_curve(&spent[0].secret.to_bytes()).expect("hash_to_curve"),
        hash_to_curve(&spent[1].secret.to_bytes()).expect("hash_to_curve"),
        hash_to_curve(&unspent[0].secret.to_bytes()).expect("hash_to_curve"),
        hash_to_curve(&unspent[1].secret.to_bytes()).expect("hash_to_curve"),
    ];
    let states = client.check_state(ys).await.expect("checkstate");
    assert_eq!(states.len(), spent.len() + unspent.len());
    assert_eq!(states[0].state, cdk07::State::Spent);
    assert_eq!(states[1].state, cdk07::State::Spent);
    assert_eq!(states[2].state, cdk07::State::Unspent);
    assert_eq!(states[3].state, cdk07::State::Unspent);
}
