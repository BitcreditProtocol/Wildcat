// ----- standard library imports
// ----- extra library imports
use bcr_wdc_swap_client::SwapClient;
use bcr_wdc_utils::keys::test_utils as key_utils;
use cashu::Amount;
// ----- local imports

#[tokio::test]
async fn swap() {
    let (server, keys_service) = bcr_wdc_swap_service::test_utils::build_test_server();
    let server_url = server.server_address().expect("address");
    let client = SwapClient::new(server_url);

    let keys_entry = key_utils::generate_keyset();
    keys_service
        .keys
        .keys
        .store(keys_entry.clone())
        .expect("store");

    let amounts = vec![Amount::from(8_u64)];
    let blinds = bcr_wdc_swap_service::utils::generate_blinds(&keys_entry.1, &amounts)
        .into_iter()
        .map(|bbb| bbb.0)
        .collect();
    let proofs = bcr_wdc_swap_service::utils::generate_proofs(&keys_entry.1, &amounts);

    client.swap(proofs, blinds).await.expect("swap");
}
