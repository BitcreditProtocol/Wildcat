// ----- standard library imports
use std::{collections::HashSet, sync::Arc, time::Duration};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    core::{signature::serialize_n_schnorr_sign_borsh_msg, BillId},
    wire::{melt as wire_melt, signatures as wire_signatures},
};
use bcr_wdc_utils::signatures as signatures_utils;
use cashu::Amount;
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence::Repository,
    TStamp,
};

// ----- end imports

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ClowderMintQuoteOnchain {
    pub clowder_quote: uuid::Uuid,
    pub cdk_quote: uuid::Uuid,
    pub address: bitcoin::Address<bitcoin::address::NetworkUnchecked>,
    pub amount: Amount,
    pub expiry: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OnchainMeltQuote {
    pub request: wire_melt::MeltQuoteOnchainRequest,
    pub expiry: u64,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Wallet: Send + Sync {
    async fn mint_quote(
        &self,
        amount: Amount,
        signed_request: wire_signatures::SignedRequestToMintFromEBillDesc,
    ) -> Result<cdk::wallet::MintQuote>;
    async fn mint(&self, quote: String) -> Result<cashu::MintQuoteState>;
    async fn keysets_info(&self, kids: &[cashu::Id]) -> Result<Vec<cashu::KeySetInfo>>;
    async fn swap_to_messages(
        &self,
        outputs: &[cashu::BlindedMessage],
    ) -> Result<Vec<cashu::BlindSignature>>;
    async fn balance(&self) -> Result<Amount>;
    async fn active_keyset(&self) -> Result<cashu::KeySetInfo>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait WildcatService: Send + Sync {
    async fn burn(&self, inputs: &[cashu::Proof]) -> Result<()>;
    async fn keyset_info(&self, kid: cashu::Id) -> Result<cashu::KeySetInfo>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ClowderService: Send + Sync {
    async fn get_sweep(&self, qid: uuid::Uuid, kid: cashu::Id) -> Result<bitcoin::Address>;
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MintQuote {
    pub qid: String,
    pub ebill_id: BillId,
}

#[derive(Clone)]
pub struct Service {
    pub wallet: Arc<dyn Wallet>,
    pub wdc: Arc<dyn WildcatService>,
    pub repo: Arc<dyn Repository>,
    pub clowder: Arc<dyn ClowderService>,
    pub signing_keys: bitcoin::secp256k1::Keypair,
    pub monitor_interval: tokio::time::Duration,
    pub quote_expiry_seconds: u64,
}

impl Service {
    pub async fn balance(&self) -> Result<Amount> {
        self.wallet.balance().await
    }

    pub async fn create_onchain_melt_quote(
        &self,
        request: wire_melt::MeltQuoteOnchainRequest,
    ) -> Result<wire_melt::MeltQuoteOnchainResponse> {
        let expiry = (chrono::Utc::now().timestamp() + self.quote_expiry_seconds as i64) as u64;
        let quote_id = Uuid::new_v4();
        tracing::info!("Creating onchain melt quote with ID {}", quote_id);
        let data = OnchainMeltQuote {
            request: request.clone(),
            expiry,
        };
        self.repo.store_onchain_melt(quote_id, data).await?;
        Ok(wire_melt::MeltQuoteOnchainResponse {
            txid: None,
            quote: quote_id,
            fee_reserve: bitcoin::Amount::ZERO,
            change: None,
            amount: bitcoin::Amount::from_sat(request.request.amount.to_sat()),
            unit: Some(request.unit),
            state: cashu::nuts::MeltQuoteState::Unpaid,
            expiry,
        })
    }

    pub async fn init_monitors_for_past_ebills(&self) -> Result<()> {
        let quotes = self.repo.list_quotes().await?;
        for quote in quotes {
            let ebill_id = quote.ebill_id.clone();
            tokio::spawn(monitor_quote(
                quote.qid,
                ebill_id,
                self.wallet.clone(),
                self.repo.clone(),
                self.monitor_interval,
            ));
        }
        Ok(())
    }

    pub async fn mint_from_ebill(
        &self,
        ebill_id: BillId,
        amount: bitcoin::Amount,
        deadline: TStamp,
    ) -> Result<cdk::wallet::MintQuote> {
        let active_kinfo = self.wallet.active_keyset().await?;
        let clowder_qid = Uuid::new_v4();
        let sweeping_address = self.clowder.get_sweep(clowder_qid, active_kinfo.id).await?;
        let request = wire_signatures::RequestToMintFromEBillDesc {
            ebill_id: ebill_id.clone(),
            deadline,
            sweeping_address: sweeping_address.to_string(),
        };
        let (content, signature) =
            serialize_n_schnorr_sign_borsh_msg(&request, &self.signing_keys)?;
        let signed_request =
            wire_signatures::SignedRequestToMintFromEBillDesc { content, signature };
        let amount = Amount::from(amount.to_sat());
        let quote = self.wallet.mint_quote(amount, signed_request).await?;
        let mint_quote = MintQuote {
            qid: quote.id.clone(),
            ebill_id: ebill_id.clone(),
        };
        self.repo.store_quote(mint_quote).await?;
        let ebill_cloned = ebill_id.clone();
        tokio::spawn(monitor_quote(
            quote.id.clone(),
            ebill_cloned,
            self.wallet.clone(),
            self.repo.clone(),
            self.monitor_interval,
        ));
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
            .fold(Amount::ZERO, |total, proof| total + proof.amount);
        let total_output = outputs
            .iter()
            .fold(Amount::ZERO, |total, proof| total + proof.amount);
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
            tokio::time::sleep(Duration::from_secs(1)).await;
            response = self.wallet.swap_to_messages(outputs).await;
            retries += 1;
        }
        response
    }
}

async fn monitor_quote(
    qid: String,
    ebill_id: BillId,
    wlt: Arc<dyn Wallet>,
    repo: Arc<dyn Repository>,
    interval: tokio::time::Duration,
) {
    loop {
        tokio::time::sleep(interval).await;
        let result = wlt.mint(qid.clone()).await;
        let Ok(status) = result else {
            tracing::warn!("Failed to mint quote {qid}: {result:?}");
            continue;
        };
        if !matches!(status, cashu::MintQuoteState::Paid) {
            tracing::info!("Quote {qid} is not paid yet, retrying...");
            continue;
        }
        let result = repo.delete_quote(qid.clone()).await;
        match result {
            Ok(_) => {
                tracing::info!("Successfully deactivated keyset for ebill {ebill_id} after minting quote {qid}");
                break;
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to delete quote {qid} after deactivating keyset for ebill {ebill_id}: {e}"
                );
            }
        };
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::persistence::MockRepository;
    use bcr_common::core_tests::generate_random_ecash_keyset;
    use bcr_wdc_utils::keys::test_utils::generate_keyset;
    use bcr_wdc_utils::signatures::test_utils as signatures_test;
    use cashu::{nut23 as cdk23, Amount};
    use mockall::predicate::*;
    use secp256k1::global::SECP256K1;
    use std::str::FromStr;

    #[tokio::test]
    async fn mint_from_ebill() {
        let sweep: bitcoin::Address =
            bitcoin::Address::from_str("1BwBExCU5qfkt1G7rqX8zDkKhhGe2p9Fdb")
                .unwrap()
                .assume_checked();
        let (info, _) = generate_random_ecash_keyset();
        let btc_amount = bitcoin::Amount::from_sat(1000);
        let amount = cashu::Amount::from(btc_amount.to_sat());
        let ebill_id =
            BillId::from_str("bitcrt285psGq4Lz4fEQwfM3We5HPznJq8p1YvRaddszFaU5dY").unwrap();
        let wdc = MockWildcatService::new();
        let mut repo = MockRepository::new();
        let mut wallet = MockWallet::new();
        let mut clowder = MockClowderService::new();
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
            .returning(move |_, _| Ok(sweep.clone()));
        let qid_cloned = mint_quote.id.clone();
        let ebill_cloned = ebill_id.clone();
        wallet
            .expect_mint_quote()
            .times(1)
            .returning(move |_, _| Ok(mint_quote.clone()));
        wallet
            .expect_active_keyset()
            .times(1)
            .returning(move || Ok(info.clone().into()));

        repo.expect_store_quote()
            .with(eq(MintQuote {
                qid: qid_cloned,
                ebill_id: ebill_cloned,
            }))
            .returning(|_| Ok(()));

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet: Arc::new(wallet),
            signing_keys,
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            monitor_interval: tokio::time::Duration::from_secs(5),
            quote_expiry_seconds: 3600,
            clowder: Arc::new(clowder),
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
        let wdc = MockWildcatService::new();
        let wallet = MockWallet::new();
        let repo = MockRepository::new();
        let clowder = MockClowderService::new();

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet: Arc::new(wallet),
            signing_keys,
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder: Arc::new(clowder),
            monitor_interval: tokio::time::Duration::from_secs(5),
            quote_expiry_seconds: 3600,
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
        let wdc = MockWildcatService::new();
        let wallet = MockWallet::new();
        let repo = MockRepository::new();
        let clowder = MockClowderService::new();

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet: Arc::new(wallet),
            signing_keys,
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder: Arc::new(clowder),
            monitor_interval: tokio::time::Duration::from_secs(5),
            quote_expiry_seconds: 3600,
        };

        let (_, keyset) = generate_keyset();
        let proofs = signatures_test::generate_proofs(&keyset, &[Amount::from(8_u64)]);

        service.redeem(&proofs, &[]).await.unwrap_err();
    }

    #[tokio::test]
    async fn redeem_unmatched_amounts() {
        let wdc = MockWildcatService::new();
        let wallet = MockWallet::new();
        let repo = MockRepository::new();
        let clowder = MockClowderService::new();

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet: Arc::new(wallet),
            signing_keys,
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder: Arc::new(clowder),
            monitor_interval: tokio::time::Duration::from_secs(5),
            quote_expiry_seconds: 3600,
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
        let wdc = MockWildcatService::new();
        let repo = MockRepository::new();
        let mut wallet = MockWallet::new();
        let clowder = MockClowderService::new();
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
        let service = Service {
            wallet: Arc::new(wallet),
            signing_keys,
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder: Arc::new(clowder),
            monitor_interval: tokio::time::Duration::from_secs(5),
            quote_expiry_seconds: 3600,
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
        let wdc = MockWildcatService::new();
        let repo = MockRepository::new();
        let mut wallet = MockWallet::new();
        let clowder = MockClowderService::new();
        wallet
            .expect_keysets_info()
            .returning(|kids| Err(Error::UnknownKeyset(kids[0])));

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet: Arc::new(wallet),
            signing_keys,
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder: Arc::new(clowder),
            monitor_interval: tokio::time::Duration::from_secs(5),
            quote_expiry_seconds: 3600,
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
