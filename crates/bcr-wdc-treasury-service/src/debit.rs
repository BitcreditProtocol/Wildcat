// ----- standard library imports
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bcr_wdc_webapi as web;
use cashu::nuts::nut00 as cdk00;
use cashu::Amount;
// ----- local imports
use crate::error::{Error, Result};

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Wallet {
    async fn mint_quote(
        &self,
        amount: Amount,
        signed_request: web::signatures::SignedRequestToMintFromEBillDesc,
    ) -> Result<cdk::wallet::MintQuote>;
}

#[derive(Clone)]
pub struct Service<Wlt> {
    pub wallet: Wlt,

    pub secp_ctx: bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::SignOnly>,
    pub signing_keys: bitcoin::secp256k1::Keypair,
}

impl<Wlt> Service<Wlt>
where
    Wlt: Wallet,
{
    pub async fn mint_from_ebill(
        &self,
        ebill_id: String,
        amount: Amount,
    ) -> Result<cdk::wallet::MintQuote> {
        let request = web::signatures::RequestToMintFromEBillDesc { ebill: ebill_id };
        let borshed = borsh::to_vec(&request).map_err(Error::BorshIO)?;
        let msg = bcr_wdc_keys::into_secp256k1_msg(&borshed);
        let signature = self.secp_ctx.sign_schnorr(&msg, &self.signing_keys);
        let signed_request = web::signatures::SignedRequestToMintFromEBillDesc {
            data: request,
            signature,
        };
        self.wallet.mint_quote(amount, signed_request).await
    }
}

#[derive(Clone)]
pub struct CDKWallet {
    wlt: cdk::wallet::Wallet,
}

impl CDKWallet {
    pub fn new(mint_url: &str, storage: &std::path::Path, seed: &[u8]) -> AnyResult<Self> {
        let storage = cdk_redb::WalletRedbDatabase::new(storage)?;
        let arced_storage = std::sync::Arc::new(storage);
        let wlt = cdk::Wallet::new(
            mint_url,
            cdk00::CurrencyUnit::Sat,
            arced_storage,
            seed,
            None,
        )?;
        Ok(Self { wlt })
    }
}

#[async_trait]
impl Wallet for CDKWallet {
    async fn mint_quote(
        &self,
        amount: Amount,
        signed_request: web::signatures::SignedRequestToMintFromEBillDesc,
    ) -> Result<cdk::wallet::MintQuote> {
        let description = serde_json::to_string(&signed_request).map_err(Error::SerdeJson)?;
        self.wlt
            .mint_quote(amount, Some(description))
            .await
            .map_err(Error::CDKWallet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_wdc_webapi as web;
    use cashu::nut04 as cdk04;
    use mockall::predicate::*;
    use mockall::*;
    use std::str::FromStr;

    #[tokio::test]
    async fn mint_from_ebill() {
        let amount = Amount::from(1000_u64);
        let ebill_id = String::from("ebill_id");
        let mut wallet = MockWallet::new();
        let ebill_id_clone = ebill_id.clone();
        let signed_request_check = predicate::function(
            move |req: &web::signatures::SignedRequestToMintFromEBillDesc| {
                req.data.ebill == ebill_id_clone
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
        let secp_ctx = bitcoin::secp256k1::Secp256k1::signing_only();
        let signing_keys = bitcoin::secp256k1::Keypair::new(&secp_ctx, &mut rand::thread_rng());
        let service = Service {
            wallet,
            secp_ctx: bitcoin::secp256k1::Secp256k1::signing_only(),
            signing_keys,
        };
        let quote = service.mint_from_ebill(ebill_id, amount).await.unwrap();
        assert_eq!(quote.id, "mint_quote_id");
    }
}
