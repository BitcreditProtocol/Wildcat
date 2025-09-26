// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_treasury_client::TreasuryClient;
use uuid::Uuid;
// ----- local imports
use crate::error::{Error, Result};
use crate::service::Wallet;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct WalletConfig {
    base_url: bcr_wdc_treasury_client::Url,
}

#[derive(Clone, Debug)]
pub struct Client {
    cl: TreasuryClient,
}

impl Client {
    pub fn new(cfg: WalletConfig) -> Self {
        let WalletConfig { base_url } = cfg;
        let cl = TreasuryClient::new(base_url);
        Self { cl }
    }
}
#[async_trait]
impl Wallet for Client {
    async fn get_blinds(
        &self,
        kid: cashu::Id,
        amount: bitcoin::Amount,
    ) -> Result<(Uuid, Vec<cashu::BlindedMessage>)> {
        let amount = cashu::Amount::from(amount.to_sat());
        self.cl
            .generate_blinds(kid, amount)
            .await
            .map_err(Error::Wallet)
    }

    async fn store_signatures(
        &self,
        rid: Uuid,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<()> {
        self.cl
            .store_signatures(rid, signatures)
            .await
            .map_err(Error::Wallet)
    }
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;

    #[derive(Clone, Debug, Default)]
    pub struct DummyWallet {}

    #[async_trait]
    impl Wallet for DummyWallet {
        async fn get_blinds(
            &self,
            kid: cashu::Id,
            amount: bitcoin::Amount,
        ) -> Result<(Uuid, Vec<cashu::BlindedMessage>)> {
            let amount = cashu::Amount::from(amount.to_sat());
            let amounts = amount.split();
            let blinds = bcr_wdc_utils::signatures::test_utils::generate_blinds(kid, &amounts)
                .into_iter()
                .map(|(b, _, _)| b)
                .collect::<Vec<_>>();
            Ok((Uuid::new_v4(), blinds))
        }

        async fn store_signatures(
            &self,
            _rid: Uuid,
            _signatures: Vec<cashu::BlindSignature>,
        ) -> Result<()> {
            Ok(())
        }
    }
}
