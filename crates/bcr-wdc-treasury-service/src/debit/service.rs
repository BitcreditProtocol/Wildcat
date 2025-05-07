// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_utils::signatures as signatures_utils;
use bcr_wdc_webapi as web;
use cashu::Amount;
use cdk::nuts::nut00 as cdk00;
use cdk::nuts::nut02 as cdk02;
use itertools::Itertools;
// ----- local imports
use crate::error::{Error, Result};

// ----- end imports

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Wallet {
    async fn mint_quote(
        &self,
        amount: Amount,
        signed_request: web::signatures::SignedRequestToMintFromEBillDesc,
    ) -> Result<cdk::wallet::MintQuote>;

    async fn get_keysets_info(&self, kids: &[cdk02::Id]) -> Result<Vec<cdk02::KeySetInfo>>;

    async fn swap_to_messages(
        &self,
        outputs: &[cdk00::BlindedMessage],
    ) -> Result<Vec<cdk00::BlindSignature>>;

    async fn balance(&self) -> Result<Amount>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ProofClient {
    async fn burn(&self, inputs: &[cdk00::Proof]) -> Result<()>;
}

#[derive(Clone)]
pub struct Service<Wlt, ProofCl> {
    pub wallet: Wlt,
    pub proof: ProofCl,
    pub signing_keys: bitcoin::secp256k1::Keypair,
}

impl<Wlt, ProofCl> Service<Wlt, ProofCl>
where
    Wlt: Wallet,
{
    pub async fn mint_from_ebill(
        &self,
        ebill_id: String,
        amount: Amount,
    ) -> Result<cdk::wallet::MintQuote> {
        let request = web::signatures::RequestToMintFromEBillDesc { ebill_id };
        let signature =
            bcr_wdc_utils::keys::schnorr_sign_borsh_msg_with_key(&request, &self.signing_keys)
                .map_err(Error::SchnorrBorshMsg)?;
        let signed_request = web::signatures::SignedRequestToMintFromEBillDesc {
            data: request,
            signature,
        };
        self.wallet.mint_quote(amount, signed_request).await
    }

    pub async fn balance(&self) -> Result<Amount> {
        self.wallet.balance().await
    }
}

impl<Wlt, ProofCl> Service<Wlt, ProofCl>
where
    Wlt: Wallet,
    ProofCl: ProofClient,
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
        // 1. outputs' keyset ID(s) are active
        let unique_ids: Vec<_> = outputs.iter().map(|p| p.keyset_id).unique().collect();
        let infos = self.wallet.get_keysets_info(&unique_ids).await?;
        for info in infos {
            if !info.active {
                return Err(Error::InactiveKeyset(info.id));
            }
        }
        // 2. burning crsat, implicitly checking proofs
        self.proof.burn(inputs).await?;

        // attempting a swap for 3 times with 1 sec pause
        let mut retries = 1_usize;
        let mut response = self.wallet.swap_to_messages(outputs).await;
        while response.is_err() && retries <= 3 {
            tracing::warn!("swap failed, attempt {}, retry in 1 second", retries);
            tokio::time::sleep(core::time::Duration::from_secs(1)).await;
            response = self.wallet.swap_to_messages(outputs).await;
            retries += 1;
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_wdc_utils::keys::test_utils::generate_keyset;
    use bcr_wdc_utils::signatures::test_utils as signatures_test;
    use bcr_wdc_webapi as web;
    use cashu::{nut00 as cdk00, nut04 as cdk04, Amount};
    use mockall::predicate::*;
    use mockall::*;
    use secp256k1::global::SECP256K1;
    use std::str::FromStr;

    #[tokio::test]
    async fn mint_from_ebill() {
        let amount = Amount::from(1000_u64);
        let ebill_id = String::from("ebill_id");
        let proof = MockProofClient::new();
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
            state: cdk04::QuoteState::Pending,
            expiry: Default::default(),
            secret_key: None,
        };
        wallet
            .expect_mint_quote()
            .with(eq(amount), signed_request_check)
            .returning(move |_, _| Ok(mint_quote.clone()));
        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet,
            signing_keys,
            proof,
        };
        let quote = service.mint_from_ebill(ebill_id, amount).await.unwrap();
        assert_eq!(quote.id, "mint_quote_id");
    }

    #[tokio::test]
    async fn redeem_no_inputs() {
        let proof = MockProofClient::new();
        let wallet = MockWallet::new();

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet,
            signing_keys,
            proof,
        };

        let (_, keyset) = generate_keyset();
        let blinds: Vec<_> = signatures_test::generate_blinds(&keyset, &vec![Amount::from(8_u64)])
            .into_iter()
            .map(|b| b.0)
            .collect();

        service.redeem(&[], &blinds).await.unwrap_err();
    }

    #[tokio::test]
    async fn redeem_no_outputs() {
        let proof = MockProofClient::new();
        let wallet = MockWallet::new();

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet,
            signing_keys,
            proof,
        };

        let (_, keyset) = generate_keyset();
        let proofs = signatures_test::generate_proofs(&keyset, &vec![Amount::from(8_u64)]);

        service.redeem(&proofs, &[]).await.unwrap_err();
    }

    #[tokio::test]
    async fn redeem_unmatched_amounts() {
        let proof = MockProofClient::new();
        let wallet = MockWallet::new();

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet,
            signing_keys,
            proof,
        };

        let (_, keyset) = generate_keyset();
        let proofs = signatures_test::generate_proofs(&keyset, &vec![Amount::from(8_u64)]);
        let blinds: Vec<_> = signatures_test::generate_blinds(&keyset, &vec![Amount::from(16_u64)])
            .into_iter()
            .map(|b| b.0)
            .collect();

        service.redeem(&proofs, &blinds).await.unwrap_err();
    }

    #[tokio::test]
    async fn redeem_inactive_keyset() {
        let proof = MockProofClient::new();
        let mut wallet = MockWallet::new();
        wallet.expect_get_keysets_info().returning(|kids| {
            let mut infos = Vec::new();
            for kid in kids {
                infos.push(cdk02::KeySetInfo {
                    id: *kid,
                    active: false,
                    unit: cdk00::CurrencyUnit::Sat,
                    input_fee_ppk: 0,
                });
            }
            Ok(infos)
        });

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet,
            signing_keys,
            proof,
        };

        let (_, keyset) = generate_keyset();
        let proofs = signatures_test::generate_proofs(&keyset, &vec![Amount::from(8_u64)]);
        let blinds: Vec<_> = signatures_test::generate_blinds(&keyset, &vec![Amount::from(16_u64)])
            .into_iter()
            .map(|b| b.0)
            .collect();

        service.redeem(&proofs, &blinds).await.unwrap_err();
    }

    #[tokio::test]
    async fn redeem_unknow_keyset() {
        let proof = MockProofClient::new();
        let mut wallet = MockWallet::new();
        wallet
            .expect_get_keysets_info()
            .returning(|kids| Err(Error::UnknownKeyset(kids[0])));

        let signing_keys = bitcoin::secp256k1::Keypair::new(SECP256K1, &mut rand::thread_rng());
        let service = Service {
            wallet,
            signing_keys,
            proof,
        };

        let (_, keyset) = generate_keyset();
        let proofs = signatures_test::generate_proofs(&keyset, &vec![Amount::from(8_u64)]);
        let blinds: Vec<_> = signatures_test::generate_blinds(&keyset, &vec![Amount::from(16_u64)])
            .into_iter()
            .map(|b| b.0)
            .collect();

        service.redeem(&proofs, &blinds).await.unwrap_err();
    }
}
