// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bcr_wdc_webapi as web;
use cashu::{
    mint_url::MintUrl,
    nuts::{nut00 as cdk00, nut02 as cdk02, nut03 as cdk03},
    Amount,
};
use cdk::wallet::{MintConnector, ReceiveOptions};
// ----- local imports
use crate::{
    debit::service::Wallet,
    error::{Error, Result},
};

// ----- end imports

#[derive(Clone, Debug, serde::Deserialize)]
pub struct CDKWalletConfig {
    pub mint_url: String,
    pub storage: std::path::PathBuf,
}

#[derive(Clone)]
pub struct CDKWallet {
    wlt: cdk::wallet::Wallet,
    client: cdk::wallet::HttpClient,
}

impl CDKWallet {
    pub async fn new(cfg: CDKWalletConfig, seed: &[u8]) -> AnyResult<Self> {
        let storage = cdk_redb::WalletRedbDatabase::new(&cfg.storage)?;
        let arced_storage = std::sync::Arc::new(storage);
        let wlt = cdk::Wallet::new(
            &cfg.mint_url,
            cdk00::CurrencyUnit::Sat,
            arced_storage,
            seed,
            None,
        )?;
        let mint_url = MintUrl::from_str(&cfg.mint_url)?;
        let client = cdk::wallet::HttpClient::new(mint_url, None);
        // make the wallet aware of the mint info
        wlt.get_mint_info().await.map_err(Error::CDKWallet)?;
        Ok(Self { wlt, client })
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
        let quote = self.wlt.mint_quote(amount, Some(description)).await?;
        Ok(quote)
    }

    async fn mint(&self, quote: String) -> Result<cashu::MintQuoteState> {
        let result = self
            .wlt
            .mint(&quote, cashu::amount::SplitTarget::default(), None)
            .await;
        match result {
            Ok(_) => Ok(cashu::MintQuoteState::Paid),
            // if unknown it must have been paid already in the past
            Err(cdk_common::Error::UnknownQuote) => Ok(cashu::MintQuoteState::Paid),
            Err(e) => Err(Error::CDKWallet(e)),
        }
    }

    async fn keysets_info(&self, kids: &[cdk02::Id]) -> Result<Vec<cdk02::KeySetInfo>> {
        let mint_infos = self.wlt.get_active_mint_keysets().await?;
        let mut infos: Vec<cdk02::KeySetInfo> = Vec::with_capacity(kids.len());
        for kid in kids {
            if let Some(info) = mint_infos.iter().find(|info| info.id == *kid) {
                infos.push(info.clone());
            } else {
                return Err(Error::UnknownKeyset(*kid));
            }
        }
        Ok(infos)
    }

    async fn swap_to_messages(
        &self,
        outputs: &[cdk00::BlindedMessage],
    ) -> Result<Vec<cdk00::BlindSignature>> {
        let total = outputs
            .iter()
            .fold(Amount::ZERO, |acc, msg| acc + msg.amount);

        self.wlt.check_all_mint_quotes().await?;

        let inputs = self.wlt.swap_from_unspent(total, None, true).await?;
        let request = cdk03::SwapRequest::new(inputs.clone(), outputs.to_vec());

        match self.client.post_swap(request).await {
            Ok(response) => Ok(response.signatures),
            Err(e) => {
                let amount = self
                    .wlt
                    .receive_proofs(inputs, ReceiveOptions::default(), None)
                    .await?;
                tracing::warn!(
                    "swap_to_messages failed with {}, restoring proofs, expected {}, received {}",
                    e,
                    total,
                    amount
                );
                Err(Error::CDKWallet(e))
            }
        }
    }

    async fn balance(&self) -> Result<Amount> {
        self.wlt.check_all_mint_quotes().await?;
        self.wlt.total_balance().await.map_err(Error::CDKWallet)
    }
}
