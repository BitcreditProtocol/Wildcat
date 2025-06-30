// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_utils::signatures as signatures_utils;
use bcr_wdc_webapi as web;
use bcr_wdc_webapi::bill::BillId;
use cashu::Amount;
use cdk::nuts::nut00 as cdk00;
use cdk::nuts::nut02 as cdk02;
use itertools::Itertools;
// ----- local imports
use crate::error::{Error, Result};

// ----- end imports

#[async_trait]
pub trait Wallet: Clone + Send {
    async fn mint_quote(
        &self,
        amount: Amount,
        signed_request: web::signatures::SignedRequestToMintFromEBillDesc,
    ) -> Result<cdk::wallet::MintQuote>;
    async fn mint(&self, quote: String) -> Result<cashu::MintQuoteState>;
    async fn keysets_info(&self, kids: &[cdk02::Id]) -> Result<Vec<cdk02::KeySetInfo>>;
    async fn swap_to_messages(
        &self,
        outputs: &[cdk00::BlindedMessage],
    ) -> Result<Vec<cdk00::BlindSignature>>;
    async fn balance(&self) -> Result<Amount>;
}

#[async_trait]
pub trait WildcatService: Clone + Send {
    async fn burn(&self, inputs: &[cdk00::Proof]) -> Result<()>;
    async fn deactivate_keyset_for_ebill(&self, ebill_id: &BillId) -> Result<cdk02::Id>;
    async fn keyset_info(&self, kid: cdk02::Id) -> Result<cdk02::KeySetInfo>;
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MintQuote {
    pub qid: String,
    pub ebill_id: BillId,
}

#[async_trait]
pub trait Repository: Clone + Send {
    async fn store_quote(&self, quote: MintQuote) -> Result<()>;
    async fn delete_quote(&self, qid: String) -> Result<()>;
    async fn list_quotes(&self) -> Result<Vec<MintQuote>>;
}

#[derive(Clone)]
pub struct Service<Wlt, WdcSrvc, Repo> {
    pub wallet: Wlt,
    pub wdc: WdcSrvc,
    pub signing_keys: bitcoin::secp256k1::Keypair,
    pub repo: Repo,
    pub monitor_interval: tokio::time::Duration,
}

impl<Wlt, WdcSrvc, Repo> Service<Wlt, WdcSrvc, Repo>
where
    Wlt: Wallet,
{
    pub async fn balance(&self) -> Result<Amount> {
        self.wallet.balance().await
    }
}

impl<Wlt, WdcSrvc, Repo> Service<Wlt, WdcSrvc, Repo>
where
    Wlt: Wallet + 'static,
    WdcSrvc: WildcatService + 'static,
    Repo: Repository + 'static,
{
    pub async fn init_monitors_for_past_ebills(&self) -> Result<()> {
        let quotes = self.repo.list_quotes().await?;
        for quote in quotes {
            let ebill_id = quote.ebill_id.clone();
            tokio::spawn(monitor_quote(
                quote.qid,
                ebill_id,
                self.wallet.clone(),
                self.repo.clone(),
                self.wdc.clone(),
                self.monitor_interval,
            ));
        }
        Ok(())
    }

    pub async fn mint_from_ebill(
        &self,
        ebill_id: BillId,
        amount: Amount,
    ) -> Result<cdk::wallet::MintQuote> {
        let request = web::signatures::RequestToMintFromEBillDesc {
            ebill_id: ebill_id.clone(),
        };
        let signature =
            bcr_wdc_utils::keys::schnorr_sign_borsh_msg_with_key(&request, &self.signing_keys)
                .map_err(Error::SchnorrBorshMsg)?;
        let signed_request = web::signatures::SignedRequestToMintFromEBillDesc {
            data: request,
            signature,
        };
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
            self.wdc.clone(),
            self.monitor_interval,
        ));
        Ok(quote)
    }
}

impl<Wlt, WdcSrvc, Repo> Service<Wlt, WdcSrvc, Repo>
where
    Wlt: Wallet,
    WdcSrvc: WildcatService,
{
    pub async fn redeem(
        &self,
        inputs: &[cdk00::Proof],
        outputs: &[cdk00::BlindedMessage],
    ) -> Result<Vec<cdk00::BlindSignature>> {
        // cheap verifications
        signatures_utils::basic_proofs_checks(inputs).map_err(Error::InvalidInput)?;
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
        let unique_ids: Vec<_> = outputs.iter().map(|p| p.keyset_id).unique().collect();
        let infos = self.wallet.keysets_info(&unique_ids).await?;
        for info in infos {
            if !info.active {
                return Err(Error::InactiveKeyset(info.id));
            }
        }
        // 2. input keysets must be inactive
        let unique_ids: Vec<_> = inputs.iter().map(|p| p.keyset_id).unique().collect();
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
            tokio::time::sleep(core::time::Duration::from_secs(1)).await;
            response = self.wallet.swap_to_messages(outputs).await;
            retries += 1;
        }
        response
    }
}

async fn monitor_quote(
    qid: String,
    ebill_id: BillId,
    wlt: impl Wallet,
    repo: impl Repository,
    wdc: impl WildcatService,
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
        let result = wdc.deactivate_keyset_for_ebill(&ebill_id).await;
        if let Err(e) = result {
            tracing::warn!(
                "Failed to deactivate keyset for ebill {ebill_id} after minting quote {qid}: {e}"
            );
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
    mockall::mock! {
        Wallet {}
        impl Clone for Wallet {
            fn clone(&self) -> Self;
        }
        #[async_trait]
        impl super::Wallet for Wallet {
            async fn mint_quote(
                &self,
                amount: Amount,
                signed_request: web::signatures::SignedRequestToMintFromEBillDesc,
            ) -> Result<cdk::wallet::MintQuote>;
            async fn mint(&self, quote: String) -> Result<cashu::MintQuoteState>;
            async fn keysets_info(&self, kids: &[cdk02::Id]) -> Result<Vec<cdk02::KeySetInfo>>;
            async fn swap_to_messages(
                &self,
                outputs: &[cdk00::BlindedMessage],
            ) -> Result<Vec<cdk00::BlindSignature>>;
            async fn balance(&self) -> Result<Amount>;
        }
    }
    mockall::mock! {
        WildcatService {}
        impl Clone for WildcatService {
            fn clone(&self) -> Self;
        }
        #[async_trait]
        impl super::WildcatService for WildcatService {
            async fn burn(&self, inputs: &[cdk00::Proof]) -> Result<()>;
            async fn deactivate_keyset_for_ebill(&self, ebill_id: &BillId) -> Result<cdk02::Id>;
            async fn keyset_info(&self, kid: cdk02::Id) -> Result<cdk02::KeySetInfo>;
        }
    }
    mockall::mock! {
        Repository {}
        impl Clone for Repository {
            fn clone(&self) -> Self;
        }
        #[async_trait]
        impl super::Repository for Repository {
            async fn store_quote(&self, quote: MintQuote) -> Result<()>;
            async fn delete_quote(&self, qid: String) -> Result<()>;
            async fn list_quotes(&self) -> Result<Vec<MintQuote>>;
        }
    }

    use super::*;
    use bcr_wdc_utils::keys::test_utils::generate_keyset;
    use bcr_wdc_utils::signatures::test_utils as signatures_test;
    use bcr_wdc_webapi as web;
    use cashu::{nut00 as cdk00, nut23 as cdk23, Amount};
    use mockall::predicate::*;
    use mockall::*;
    use secp256k1::global::SECP256K1;
    use std::str::FromStr;

    #[tokio::test]
    async fn mint_from_ebill() {
        let amount = Amount::from(1000_u64);
        let ebill_id =
            BillId::from_str("bitcrt285psGq4Lz4fEQwfM3We5HPznJq8p1YvRaddszFaU5dY").unwrap();
        let mut wdc = MockWildcatService::new();
        let mut repo = MockRepository::new();
        let mut wallet = MockWallet::new();
        let ebill_id_clone = ebill_id.clone();
        let signed_request_check = predicate::function(
            move |req: &web::signatures::SignedRequestToMintFromEBillDesc| {
                req.data.ebill_id == ebill_id_clone
            },
        );
        let mint_quote = cdk::wallet::MintQuote {
            id: String::from("mint_quote_id"),
            mint_url: cdk_common::mint_url::MintUrl::from_str("http://test_mint_url.com:3338")
                .unwrap(),
            amount,
            unit: cdk00::CurrencyUnit::Sat,
            request: Default::default(),
            state: cdk23::QuoteState::Pending,
            expiry: Default::default(),
            secret_key: None,
        };
        let qid_cloned = mint_quote.id.clone();
        let ebill_cloned = ebill_id.clone();
        wallet
            .expect_mint_quote()
            .with(eq(amount), signed_request_check)
            .returning(move |_, _| Ok(mint_quote.clone()));
        let wallet_cloned = MockWallet::new();
        wallet.expect_clone().return_once(move || wallet_cloned);

        repo.expect_store_quote()
            .with(eq(MintQuote {
                qid: qid_cloned,
                ebill_id: ebill_cloned,
            }))
            .returning(|_| Ok(()));
        let repo_cloned = MockRepository::new();
        repo.expect_clone().return_once(move || repo_cloned);

        let wdc_cloned = MockWildcatService::new();
        wdc.expect_clone().return_once(move || wdc_cloned);

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet,
            signing_keys,
            wdc,
            repo,
            monitor_interval: tokio::time::Duration::from_secs(5),
        };
        let quote = service.mint_from_ebill(ebill_id, amount).await.unwrap();
        assert_eq!(quote.id, "mint_quote_id");
    }

    #[tokio::test]
    async fn redeem_no_inputs() {
        let wdc = MockWildcatService::new();
        let wallet = MockWallet::new();
        let repo = MockRepository::new();

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet,
            signing_keys,
            wdc,
            repo,
            monitor_interval: tokio::time::Duration::from_secs(5),
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

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet,
            signing_keys,
            wdc,
            repo,
            monitor_interval: tokio::time::Duration::from_secs(5),
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

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet,
            signing_keys,
            wdc,
            repo,
            monitor_interval: tokio::time::Duration::from_secs(5),
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
        wallet.expect_keysets_info().returning(|kids| {
            let mut infos = Vec::new();
            for kid in kids {
                infos.push(cdk02::KeySetInfo {
                    id: *kid,
                    active: false,
                    unit: cdk00::CurrencyUnit::Sat,
                    input_fee_ppk: 0,
                    final_expiry: None,
                });
            }
            Ok(infos)
        });

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet,
            signing_keys,
            wdc,
            repo,
            monitor_interval: tokio::time::Duration::from_secs(5),
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
        wallet
            .expect_keysets_info()
            .returning(|kids| Err(Error::UnknownKeyset(kids[0])));

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet,
            signing_keys,
            wdc,
            repo,
            monitor_interval: tokio::time::Duration::from_secs(5),
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
