// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    client::{
        core::Client as CoreClient,
        ebill::Client as EBillClient,
        treasury::{Client as TreasuryClient, Error as TreasuryError},
    },
    core::BillId,
    wire::{bill as wire_bill, quotes as wire_quotes},
};
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::{
    error::{Error, Result},
    service::{MintingStatus, WdcClient},
};

#[derive(Debug, Clone)]
pub struct WildcatCl {
    pub core: CoreClient,
    pub treasury: TreasuryClient,
    pub ebill: EBillClient,
}

#[async_trait]
impl WdcClient for WildcatCl {
    async fn get_keyset_with_redemption_date(
        &self,
        redemption_date: chrono::NaiveDate,
    ) -> Result<cashu::Id> {
        let kid = self.core.keys_for_expiration(redemption_date).await?;
        Ok(kid)
    }

    async fn get_keys(&self, keyset_id: cashu::Id) -> Result<cashu::KeySet> {
        let keyset = self.core.keys(keyset_id).await?;
        Ok(keyset)
    }

    async fn add_new_mint_operation(
        &self,
        qid: Uuid,
        kid: cashu::Id,
        pk: cashu::PublicKey,
        target: cashu::Amount,
        bill_id: BillId,
    ) -> Result<()> {
        self.treasury
            .new_mint_operation(qid, kid, pk, target, bill_id)
            .await?;
        Ok(())
    }

    async fn sign(&self, msg: &cashu::BlindedMessage) -> Result<cashu::BlindSignature> {
        let signatures = self.core.sign(msg).await?;
        Ok(signatures)
    }

    async fn get_minting_status(&self, qid: Uuid) -> Result<MintingStatus> {
        let response = self.treasury.mint_operation_status(qid).await;
        match response {
            Ok(status) => Ok(MintingStatus::Enabled(status.current)),
            Err(TreasuryError::MintOpNotFound(_)) => Ok(MintingStatus::Disabled),
            Err(e) => Err(Error::TreasuryClient(e)),
        }
    }

    async fn validate_and_decrypt_shared_bill(
        &self,
        shared_bill: &wire_quotes::SharedBill,
    ) -> Result<wire_quotes::BillInfo> {
        let ebill = self
            .ebill
            .validate_and_decrypt_shared_bill(shared_bill)
            .await?;
        Ok(ebill)
    }

    async fn get_ebill(&self, bid: BillId) -> Result<wire_bill::BitcreditBill> {
        let ebill = self.ebill.get_bill(&bid).await?;
        Ok(ebill)
    }
}
