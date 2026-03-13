// ----- standard library imports
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
// ----- extra library imports
use bcr_common::{
    cashu,
    cdk::{self, wallet::MintConnector},
    core::{signature::serialize_n_schnorr_sign_borsh_msg, BillId},
    wire::{
        clowder::messages as clowder_messages, melt as wire_melt, mint as wire_mint,
        signatures as wire_signatures,
    },
};
use bcr_wdc_utils::signatures as signatures_utils;
use uuid::Uuid;
// ----- local imports
use crate::{
    debit::{ClowderClient, MintStatus, OnChainMintOperation, Repository, Wallet, WildcatClient},
    error::{Error, Result},
    TStamp,
};

// ----- end imports

fn create_clowder_melt_bolt11(amount: cashu::Amount) -> cashu::lightning_invoice::Bolt11Invoice {
    use bitcoin::hashes::{sha256, Hash};
    use cashu::lightning_invoice as ln;

    // Random unused values
    let payment_hash = sha256::Hash::hash(&rand::random::<[u8; 32]>());
    let payment_secret = ln::PaymentSecret(rand::random());
    let sk = secp256k1::SecretKey::new(&mut rand::thread_rng());
    let description = format!("clowder:melt:{}", u64::from(amount));

    ln::InvoiceBuilder::new(ln::Currency::Bitcoin)
        .description(description)
        .payment_hash(payment_hash)
        .payment_secret(payment_secret)
        .current_timestamp()
        .amount_milli_satoshis(u64::from(amount) * 1000) // msat
        .min_final_cltv_expiry_delta(144)
        .build_signed(|hash| secp256k1::global::SECP256K1.sign_ecdsa_recoverable(hash, &sk))
        .unwrap()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OnchainMeltQuote {
    pub request: wire_melt::MeltQuoteOnchainRequest,
    pub expiry: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MintQuote {
    pub qid: String,
    pub ebill_id: BillId,
    pub clowder_qid: uuid::Uuid,
    pub mint_complete: bool,
}

#[derive(Clone)]
pub struct Service {
    pub wallet: Arc<dyn Wallet>,
    pub wdc: Arc<dyn WildcatClient>,
    pub repo: Arc<dyn Repository>,
    pub clowder_cl: Arc<dyn ClowderClient>,
    pub signing_keys: bitcoin::secp256k1::Keypair,
    pub monitor_interval: chrono::Duration,
    pub quote_expiry: chrono::Duration,
    pub cancel: tokio_util::sync::CancellationToken,
    pub hndls: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    pub dbmint: cdk::wallet::HttpClient,
    pub min_mint_threshold: bitcoin::Amount,
}

impl Service {
    const MIN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

    pub async fn new_onchain_mintop(
        &self,
        request: wire_mint::OnchainMintQuoteRequest,
        now: TStamp,
    ) -> Result<wire_mint::OnchainMintQuoteResponse> {
        bcr_wdc_utils::signatures::basic_blinds_checks(&request.blinded_messages)
            .map_err(|e| Error::InvalidInput(e.to_string()))?;
        let qid = Uuid::new_v4();
        let blinds_camount = request
            .blinded_messages
            .iter()
            .fold(cashu::Amount::ZERO, |total, b| total + b.amount);
        let blinds_amount = bitcoin::Amount::from_sat(blinds_camount.into());
        if blinds_amount < self.min_mint_threshold {
            return Err(Error::InvalidInput(String::from("mint amount too low")));
        }
        let kid = self.wdc.get_active_keyset().await?;
        let same_kid = request.blinded_messages.iter().all(|b| b.keyset_id == kid);
        if !same_kid {
            return Err(Error::InvalidInput(String::from("invalid keyset id")));
        }
        let address = self
            .clowder_cl
            .request_onchain_mint_address(qid, kid)
            .await?;
        let expiry = now + self.quote_expiry;
        let mintop = OnChainMintOperation {
            qid,
            target: blinds_camount,
            recipient: address.as_unchecked().clone(),
            expiry,
            status: MintStatus::Pending(request.blinded_messages.clone()),
        };
        self.repo.store_onchain_mintop(mintop).await?;
        let body = wire_mint::OnchainMintQuoteResponseBody {
            quote: qid,
            address: address.to_string(),
            payment_amount: bitcoin::Amount::from_sat(blinds_camount.into()),
            blinded_messages: request.blinded_messages,
            expiry: expiry.timestamp().max(0) as u64,
        };

        let (content, commitment) = self.clowder_cl.sign_onchain_mint_response(&body).await?;
        let hndlr = tokio::spawn(monitor_onchain_mintop(
            qid,
            kid,
            self.monitor_interval.to_std().unwrap_or(Self::MIN_INTERVAL),
            self.quote_expiry,
            self.wdc.clone(),
            self.clowder_cl.clone(),
            self.repo.clone(),
            self.cancel.clone(),
        ));
        self.hndls.lock().unwrap().push(hndlr);
        let response = wire_mint::OnchainMintQuoteResponse {
            commitment,
            content,
        };
        Ok(response)
    }

    pub async fn balance(&self) -> Result<cashu::Amount> {
        self.wallet.balance().await
    }

    pub async fn stop(&self) -> Result<()> {
        self.cancel.cancel();
        loop {
            let next = self.hndls.lock().unwrap().pop();
            let Some(handle) = next else { return Ok(()) };
            handle.await.map_err(|e| Error::Internal(e.to_string()))?;
        }
    }

    pub async fn create_onchain_melt_quote(
        &self,
        request: wire_melt::MeltQuoteOnchainRequest,
    ) -> Result<wire_melt::MeltQuoteOnchainResponse> {
        let expiry = (chrono::Utc::now() + self.quote_expiry).timestamp().max(0) as u64;
        let amount = cashu::Amount::from(request.request.amount.to_sat());

        let bolt11 = create_clowder_melt_bolt11(amount);
        let melt_quote_req = cashu::MeltQuoteBolt11Request {
            request: bolt11,
            unit: cashu::CurrencyUnit::Sat,
            options: None,
        };

        let cdk_quote = match self.dbmint.post_melt_quote(melt_quote_req).await {
            Ok(resp) => {
                tracing::info!("CDK melt quote created: {}", resp.quote);
                Uuid::parse_str(&resp.quote)
                    .map_err(|_| Error::InvalidInput("Invalid CDK quote ID".into()))?
            }
            Err(e) => {
                tracing::error!("Failed to create CDK melt quote: {:?}", e);
                return Err(Error::Internal(format!("CDK quote creation failed: {}", e)));
            }
        };

        let data = OnchainMeltQuote {
            request: request.clone(),
            expiry,
        };
        self.repo.store_onchain_melt(cdk_quote, data).await?;

        Ok(wire_melt::MeltQuoteOnchainResponse {
            txid: None,
            quote: cdk_quote,
            fee_reserve: bitcoin::Amount::ZERO,
            change: None,
            amount: request.request.amount,
            unit: Some(request.unit),
            state: cashu::nuts::MeltQuoteState::Unpaid,
            expiry,
        })
    }

    pub async fn init_monitors_for_past_ebills(&self) -> Result<()> {
        let quotes = self.repo.list_quotes().await?;
        for quote in quotes {
            if quote.mint_complete {
                continue;
            }
            let hndl = tokio::spawn(monitor_quote(
                quote,
                self.wallet.clone(),
                self.repo.clone(),
                self.clowder_cl.clone(),
                self.monitor_interval.to_std().unwrap_or(Self::MIN_INTERVAL),
                self.cancel.clone(),
            ));
            self.hndls.lock().unwrap().push(hndl);
        }
        Ok(())
    }

    pub async fn mint_from_ebill(
        &self,
        ebill_id: BillId,
        amount: bitcoin::Amount,
        deadline: TStamp,
    ) -> Result<cdk::wallet::MintQuote> {
        let clowder_qid = Uuid::new_v4();
        let sweeping_address = self.clowder_cl.get_sweep(clowder_qid).await?;
        let request = wire_signatures::RequestToMintFromEBillDesc {
            ebill_id: ebill_id.clone(),
            deadline,
            sweeping_address: sweeping_address.to_string(),
        };
        let (content, signature) =
            serialize_n_schnorr_sign_borsh_msg(&request, &self.signing_keys)?;
        let signed_request =
            wire_signatures::SignedRequestToMintFromEBillDesc { content, signature };
        let amount = cashu::Amount::from(amount.to_sat());
        let quote = self.wallet.mint_quote(amount, signed_request).await?;
        let mint_quote = MintQuote {
            qid: quote.id.clone(),
            ebill_id: ebill_id.clone(),
            clowder_qid,
            mint_complete: false,
        };
        self.repo.store_quote(mint_quote.clone()).await?;
        let hndl = tokio::spawn(monitor_quote(
            mint_quote,
            self.wallet.clone(),
            self.repo.clone(),
            self.clowder_cl.clone(),
            self.monitor_interval.to_std().unwrap_or(Self::MIN_INTERVAL),
            self.cancel.clone(),
        ));
        self.hndls.lock().unwrap().push(hndl);
        Ok(quote)
    }

    pub async fn redeem(
        &self,
        inputs: &[cashu::Proof],
        outputs: &[cashu::BlindedMessage],
    ) -> Result<Vec<cashu::BlindSignature>> {
        // cheap verifications
        signatures_utils::basic_proofs_checks(inputs)
            .map_err(|e| Error::InvalidInput(e.to_string()))?;
        signatures_utils::basic_blinds_checks(outputs).map_err(Error::InvalidOutput)?;
        // 3. inputs and outputs have equal amounts
        let total_input = inputs
            .iter()
            .fold(cashu::Amount::ZERO, |total, proof| total + proof.amount);
        let total_output = outputs
            .iter()
            .fold(cashu::Amount::ZERO, |total, proof| total + proof.amount);
        if total_input != total_output {
            return Err(Error::UnmatchingAmount(total_input, total_output));
        }
        // expensive verifications
        // 1. output keysets must be active
        let unique_ids: HashSet<_> = outputs.iter().map(|p| p.keyset_id).collect();
        let unique_ids: Vec<_> = unique_ids.into_iter().collect();
        let infos = self.wallet.keysets_info(&unique_ids).await?;
        for info in infos {
            if !info.active {
                return Err(Error::InactiveKeyset(info.id));
            }
        }
        // 2. input keysets must be inactive
        let unique_ids: HashSet<_> = inputs.iter().map(|p| p.keyset_id).collect();
        let unique_ids: Vec<_> = unique_ids.into_iter().collect();
        for id in unique_ids {
            let info = self.wdc.keyset_info(id).await?;
            if info.active {
                return Err(Error::ActiveKeyset(info.id));
            }
        }
        // 3. do we have enough balance?
        let balance = self.wallet.balance().await?;
        if balance < total_input {
            return Err(Error::UnmatchingAmount(total_input, balance));
        }
        // 4. burning crsat, implicitly checking proofs
        self.wdc.burn(inputs).await?;

        // attempting a swap for 3 times with 1 sec pause
        let mut retries = 1_usize;
        let mut response = self.wallet.swap_to_messages(outputs).await;
        while response.is_err() && retries <= 3 {
            tracing::warn!("swap failed, attempt {retries}, retry in 1 second");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            response = self.wallet.swap_to_messages(outputs).await;
            retries += 1;
        }
        response
    }

    pub async fn is_ebill_payment_minted(&self, ebill: BillId) -> Result<bool> {
        let quotes = self.repo.list_quotes().await?;
        let quote = quotes.into_iter().find(|q| q.ebill_id == ebill);
        let Some(quote) = quote else {
            return Err(Error::EBillNotFound(ebill.to_string()));
        };
        Ok(quote.mint_complete)
    }
}

async fn monitor_onchain_mintop(
    qid: Uuid,
    kid: cashu::Id,
    interval: std::time::Duration,
    expiry: chrono::Duration,
    wildcat_cl: Arc<dyn WildcatClient>,
    clowder_cl: Arc<dyn ClowderClient>,
    repo: Arc<dyn Repository>,
    cancel: tokio_util::sync::CancellationToken,
) {
    let mut total = chrono::Duration::zero();
    while (total < expiry) && !cancel.is_cancelled() {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("Monitor for onchain mintop {qid} cancelled");
                return;
            }
            _ = tokio::time::sleep(interval) => {}
        }
        total = total + chrono::Duration::from_std(interval).unwrap_or_default();
        let amount = match clowder_cl.verify_onchain_mint_payment(qid, kid).await {
            Ok(amount) => amount,
            Err(e) => {
                tracing::warn!("Failed to verify onchain mintop {qid}: {e}");
                continue;
            }
        };
        if amount == bitcoin::Amount::ZERO {
            tracing::info!("Onchain mintop {qid} not received yet");
            continue;
        }
        let mintop = match repo.load_onchain_mintop(qid).await {
            Ok(mintop) => mintop,
            Err(e) => {
                tracing::error!("Failed to load onchain mintop {qid} from repo: {e}");
                continue;
            }
        };
        let camount = cashu::Amount::from(amount.to_sat());
        if camount < mintop.target {
            tracing::info!(
                "Onchain mintop {qid} underpaid, {camount} < {}",
                mintop.target
            );
            continue;
        }
        let MintStatus::Pending(blinds) = mintop.status else {
            tracing::info!("Onchain mintop {qid} already processed");
            return;
        };
        let signatures = match wildcat_cl.sign(blinds).await {
            Ok(signatures) => signatures,
            Err(e) => {
                tracing::warn!("onchain mintop {qid}, failed sign {e}");
                continue;
            }
        };
        match clowder_cl.mint_onchain(qid, kid, signatures.clone()).await {
            Ok(_) => {
                tracing::info!("Onchain mintop {qid} successfully minted on clowder");
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to mint on clowder for onchain mintop {qid}: {e}, retrying in {} second", interval.as_secs()
                );
                tokio::time::sleep(interval).await;
                continue;
            }
        };
        let new_status = MintStatus::Paid(signatures);
        match repo.update_onchain_mintop_status(qid, new_status).await {
            Ok(_) => {
                tracing::info!("Onchain mintop {qid} marked as paid in repo");
                return;
            }
            Err(e) => {
                tracing::error!("Failed to update onchain mintop {qid} status in repo: {e}");
                continue;
            }
        };
    }
    tracing::info!("Monitor for onchain mintop {qid} expired, exiting");
}

async fn monitor_quote(
    mut quote: MintQuote,
    wlt: Arc<dyn Wallet>,
    repo: Arc<dyn Repository>,
    clowder: Arc<dyn ClowderClient>,
    interval: tokio::time::Duration,
    cancel: tokio_util::sync::CancellationToken,
) {
    let qid = quote.qid.clone();
    let ebill_id = quote.ebill_id.clone();
    let clowder_qid = quote.clowder_qid;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("Monitor for quote {qid} cancelled");
                return;
            }
            _ = tokio::time::sleep(interval) => {}
        }
        let result = wlt.mint(qid.clone()).await;
        let Ok(status) = result else {
            tracing::warn!("Failed to mint quote {qid}: {result:?}");
            continue;
        };
        if !matches!(status, cashu::MintQuoteState::Paid) {
            tracing::info!("Quote {qid} is not paid yet, retrying...");
            continue;
        }
        break;
    }
    let req = clowder_messages::BillPaymentRequest {
        bill_id: ebill_id.clone(),
        payment_clowder_quote: clowder_qid,
    };
    let resp = clowder_messages::BillPaymentResponse {};
    if let Err(e) = clowder.pay_bill(req, resp).await {
        tracing::warn!("Failed to call clowder pay_bill for ebill {ebill_id}: {e}");
    }
    quote.mint_complete = true;
    let result = repo.update_quote(quote).await;
    match result {
        Ok(_) => {
            tracing::info!(
                "Successfully minted debit sat for ebill {ebill_id} after minting quote {qid}"
            );
        }
        Err(e) => {
            tracing::error!("Failed to update quote {qid} after minting ebill {ebill_id}: {e}");
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debit::{MockClowderClient, MockRepository, MockWallet, MockWildcatClient};
    use bcr_common::cdk_common;
    use bcr_wdc_utils::keys::test_utils::generate_keyset;
    use bcr_wdc_utils::signatures::test_utils as signatures_test;
    use cashu::{nut23 as cdk23, Amount};
    use secp256k1::global::SECP256K1;
    use std::str::FromStr;

    #[tokio::test]
    async fn mint_from_ebill() {
        let sweep: bitcoin::Address =
            bitcoin::Address::from_str("1BwBExCU5qfkt1G7rqX8zDkKhhGe2p9Fdb")
                .unwrap()
                .assume_checked();
        let btc_amount = bitcoin::Amount::from_sat(1000);
        let amount = cashu::Amount::from(btc_amount.to_sat());
        let ebill_id =
            BillId::from_str("bitcrt285psGq4Lz4fEQwfM3We5HPznJq8p1YvRaddszFaU5dY").unwrap();
        let wdc = MockWildcatClient::new();
        let mut repo = MockRepository::new();
        let mut wallet = MockWallet::new();
        let mut clowder = MockClowderClient::new();
        let mint_quote = cdk::wallet::MintQuote {
            id: String::from("mint_quote_id"),
            mint_url: cdk_common::mint_url::MintUrl::from_str("http://test_mint_url.com:3338")
                .unwrap(),
            amount: Some(amount),
            amount_paid: amount,
            amount_issued: amount,
            payment_method: cashu::PaymentMethod::Bolt11,
            unit: cashu::CurrencyUnit::Sat,
            request: Default::default(),
            state: cdk23::QuoteState::Paid,
            expiry: Default::default(),
            secret_key: None,
        };
        clowder
            .expect_get_sweep()
            .times(1)
            .returning(move |_| Ok(sweep.clone()));
        let qid_cloned = mint_quote.id.clone();
        let ebill_cloned = ebill_id.clone();
        wallet
            .expect_mint_quote()
            .times(1)
            .returning(move |_, _| Ok(mint_quote.clone()));

        repo.expect_store_quote()
            .withf(move |q| q.qid == qid_cloned && q.ebill_id == ebill_cloned)
            .returning(|_| Ok(()));

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let cdk_mint = cdk::wallet::HttpClient::new(
            cashu::MintUrl::from_str("http://test_mint_url.com:3338").unwrap(),
        );
        let service = Service {
            wallet: Arc::new(wallet),
            signing_keys,
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            monitor_interval: chrono::Duration::seconds(5),
            quote_expiry: chrono::Duration::seconds(3600),
            clowder_cl: Arc::new(clowder),
            cancel: tokio_util::sync::CancellationToken::new(),
            hndls: Arc::new(Mutex::new(Vec::new())),
            dbmint: cdk_mint,
            min_mint_threshold: bitcoin::Amount::ZERO,
        };
        let quote = service
            .mint_from_ebill(
                ebill_id,
                btc_amount,
                TStamp::from_str("2026-01-01T00:00:00Z").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(quote.id, "mint_quote_id");
    }

    #[tokio::test]
    async fn redeem_no_inputs() {
        let wdc = MockWildcatClient::new();
        let wallet = MockWallet::new();
        let repo = MockRepository::new();
        let clowder = MockClowderClient::new();

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let cdk_mint = cdk::wallet::HttpClient::new(
            cashu::MintUrl::from_str("http://test_mint_url.com:3338").unwrap(),
        );
        let service = Service {
            wallet: Arc::new(wallet),
            signing_keys,
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            monitor_interval: chrono::Duration::seconds(5),
            quote_expiry: chrono::Duration::seconds(3600),
            cancel: tokio_util::sync::CancellationToken::new(),
            hndls: Arc::new(Mutex::new(Vec::new())),
            dbmint: cdk_mint,
            min_mint_threshold: bitcoin::Amount::ZERO,
        };
        let (_, keyset) = generate_keyset();
        let blinds: Vec<_> = signatures_test::generate_blinds(keyset.id, &[Amount::from(8_u64)])
            .into_iter()
            .map(|b| b.0)
            .collect();

        service.redeem(&[], &blinds).await.unwrap_err();
    }

    #[tokio::test]
    async fn redeem_no_outputs() {
        let wdc = MockWildcatClient::new();
        let wallet = MockWallet::new();
        let repo = MockRepository::new();
        let clowder = MockClowderClient::new();

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let cdk_mint = cdk::wallet::HttpClient::new(
            cashu::MintUrl::from_str("http://test_mint_url.com:3338").unwrap(),
        );
        let service = Service {
            wallet: Arc::new(wallet),
            signing_keys,
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            monitor_interval: chrono::Duration::seconds(5),
            quote_expiry: chrono::Duration::seconds(3600),
            cancel: tokio_util::sync::CancellationToken::new(),
            hndls: Arc::new(Mutex::new(Vec::new())),
            dbmint: cdk_mint,
            min_mint_threshold: bitcoin::Amount::ZERO,
        };
        let (_, keyset) = generate_keyset();
        let proofs = signatures_test::generate_proofs(&keyset, &[Amount::from(8_u64)]);
        service.redeem(&proofs, &[]).await.unwrap_err();
    }

    #[tokio::test]
    async fn redeem_unmatched_amounts() {
        let wdc = MockWildcatClient::new();
        let wallet = MockWallet::new();
        let repo = MockRepository::new();
        let clowder = MockClowderClient::new();

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let cdk_mint = cdk::wallet::HttpClient::new(
            cashu::MintUrl::from_str("http://test_mint_url.com:3338").unwrap(),
        );
        let service = Service {
            wallet: Arc::new(wallet),
            signing_keys,
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            monitor_interval: chrono::Duration::seconds(5),
            quote_expiry: chrono::Duration::seconds(3600),
            cancel: tokio_util::sync::CancellationToken::new(),
            hndls: Arc::new(Mutex::new(Vec::new())),
            dbmint: cdk_mint,
            min_mint_threshold: bitcoin::Amount::ZERO,
        };
        let (_, keyset) = generate_keyset();
        let proofs = signatures_test::generate_proofs(&keyset, &[Amount::from(8_u64)]);
        let blinds: Vec<_> = signatures_test::generate_blinds(keyset.id, &[Amount::from(16_u64)])
            .into_iter()
            .map(|b| b.0)
            .collect();
        service.redeem(&proofs, &blinds).await.unwrap_err();
    }

    #[tokio::test]
    async fn redeem_inactive_keyset() {
        let wdc = MockWildcatClient::new();
        let repo = MockRepository::new();
        let mut wallet = MockWallet::new();
        let clowder = MockClowderClient::new();
        wallet.expect_keysets_info().returning(|kids| {
            let mut infos = Vec::new();
            for kid in kids {
                infos.push(cashu::KeySetInfo {
                    id: *kid,
                    active: false,
                    unit: cashu::CurrencyUnit::Sat,
                    input_fee_ppk: 0,
                    final_expiry: None,
                });
            }
            Ok(infos)
        });

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let cdk_mint = cdk::wallet::HttpClient::new(
            cashu::MintUrl::from_str("http://test_mint_url.com:3338").unwrap(),
        );
        let service = Service {
            wallet: Arc::new(wallet),
            signing_keys,
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            monitor_interval: chrono::Duration::seconds(5),
            quote_expiry: chrono::Duration::seconds(3600),
            cancel: tokio_util::sync::CancellationToken::new(),
            hndls: Arc::new(Mutex::new(Vec::new())),
            dbmint: cdk_mint,
            min_mint_threshold: bitcoin::Amount::ZERO,
        };
        let (_, keyset) = generate_keyset();
        let proofs = signatures_test::generate_proofs(&keyset, &[Amount::from(8_u64)]);
        let blinds: Vec<_> = signatures_test::generate_blinds(keyset.id, &[Amount::from(16_u64)])
            .into_iter()
            .map(|b| b.0)
            .collect();
        service.redeem(&proofs, &blinds).await.unwrap_err();
    }

    #[tokio::test]
    async fn redeem_unknow_keyset() {
        let wdc = MockWildcatClient::new();
        let repo = MockRepository::new();
        let mut wallet = MockWallet::new();
        let clowder = MockClowderClient::new();
        wallet
            .expect_keysets_info()
            .returning(|kids| Err(Error::UnknownKeyset(kids[0])));

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let cdk_mint = cdk::wallet::HttpClient::new(
            cashu::MintUrl::from_str("http://test_mint_url.com:3338").unwrap(),
        );
        let service = Service {
            wallet: Arc::new(wallet),
            signing_keys,
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            monitor_interval: chrono::Duration::seconds(5),
            quote_expiry: chrono::Duration::seconds(3600),
            cancel: tokio_util::sync::CancellationToken::new(),
            hndls: Arc::new(Mutex::new(Vec::new())),
            dbmint: cdk_mint,
            min_mint_threshold: bitcoin::Amount::ZERO,
        };

        let (_, keyset) = generate_keyset();
        let proofs = signatures_test::generate_proofs(&keyset, &[Amount::from(8_u64)]);
        let blinds: Vec<_> = signatures_test::generate_blinds(keyset.id, &[Amount::from(16_u64)])
            .into_iter()
            .map(|b| b.0)
            .collect();

        service.redeem(&proofs, &blinds).await.unwrap_err();
    }
}
