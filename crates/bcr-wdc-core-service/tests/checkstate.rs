// ----- standard library imports
// ----- extra library imports
use bcr_common::{cashu, client::core::Client as CoreClient, core_tests};
use bcr_wdc_utils::MintKeysEntry;
// ----- local imports

// ----- end imports

#[tokio::test]
async fn checkstate() {
    let (server, controller) = bcr_wdc_core_service::test_utils::build_test_server(None).await;
    let server_url = server.server_address().expect("address");
    let corecl = CoreClient::new(server_url.clone());

    let (mut info, set) = core_tests::generate_random_ecash_keyset();
    info.active = false;
    let entry = MintKeysEntry {
        id: info.id,
        unit: info.unit.clone(),
        active: info.active,
        valid_from: info.valid_from,
        derivation_path: info.derivation_path.clone(),
        derivation_path_index: info.derivation_path_index,
        amounts: info.amounts.clone(),
        input_fee_ppk: info.input_fee_ppk,
        final_expiry: info.final_expiry,
        keys: set.keys.clone(),
    };
    controller
        .keys
        .repository
        .keys_store(entry)
        .await
        .expect("store");

    let amounts = vec![cashu::Amount::from(8_u64), cashu::Amount::from(16_u64)];
    let spent = core_tests::generate_random_ecash_proofs(&set, &amounts);

    corecl.burn(spent.clone()).await.expect("burn");

    let amounts = vec![cashu::Amount::from(32_u64), cashu::Amount::from(64_u64)];
    let unspent = core_tests::generate_random_ecash_proofs(&set, &amounts);

    let ys = vec![
        cashu::dhke::hash_to_curve(&spent[0].secret.to_bytes()).expect("hash_to_curve"),
        cashu::dhke::hash_to_curve(&spent[1].secret.to_bytes()).expect("hash_to_curve"),
        cashu::dhke::hash_to_curve(&unspent[0].secret.to_bytes()).expect("hash_to_curve"),
        cashu::dhke::hash_to_curve(&unspent[1].secret.to_bytes()).expect("hash_to_curve"),
    ];
    let states = corecl.check_state(ys).await.expect("checkstate");
    assert_eq!(states.len(), spent.len() + unspent.len());
    assert_eq!(states[0].state, cashu::State::Spent);
    assert_eq!(states[1].state, cashu::State::Spent);
    assert_eq!(states[2].state, cashu::State::Unspent);
    assert_eq!(states[3].state, cashu::State::Unspent);
}
