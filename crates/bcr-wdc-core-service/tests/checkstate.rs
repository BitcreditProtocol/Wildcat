// ----- standard library imports
// ----- extra library imports
use bcr_common::{cashu, client::core::Client as CoreClient};
use bcr_wdc_utils::{keys::test_utils as keys_test, signatures::test_utils as signatures_test};
// ----- local imports

// ----- end imports

#[tokio::test]
async fn checkstate() {
    let (server, controller) = bcr_wdc_core_service::test_utils::build_test_server(None).await;
    let server_url = server.server_address().expect("address");
    let client = CoreClient::new(server_url);

    let mut keys_entry = keys_test::generate_keyset();
    keys_entry.0.active = false;
    controller
        .keys
        .keys
        .store(keys_entry.clone())
        .await
        .expect("store");

    let amounts = vec![cashu::Amount::from(8_u64), cashu::Amount::from(16_u64)];
    let spent = signatures_test::generate_proofs(&keys_entry.1, &amounts);

    client.burn(spent.clone()).await.expect("burn");

    let amounts = vec![cashu::Amount::from(32_u64), cashu::Amount::from(64_u64)];
    let unspent = signatures_test::generate_proofs(&keys_entry.1, &amounts);

    let ys = vec![
        cashu::dhke::hash_to_curve(&spent[0].secret.to_bytes()).expect("hash_to_curve"),
        cashu::dhke::hash_to_curve(&spent[1].secret.to_bytes()).expect("hash_to_curve"),
        cashu::dhke::hash_to_curve(&unspent[0].secret.to_bytes()).expect("hash_to_curve"),
        cashu::dhke::hash_to_curve(&unspent[1].secret.to_bytes()).expect("hash_to_curve"),
    ];
    let states = client.check_state(ys).await.expect("checkstate");
    assert_eq!(states.len(), spent.len() + unspent.len());
    assert_eq!(states[0].state, cashu::State::Spent);
    assert_eq!(states[1].state, cashu::State::Spent);
    assert_eq!(states[2].state, cashu::State::Unspent);
    assert_eq!(states[3].state, cashu::State::Unspent);
}
