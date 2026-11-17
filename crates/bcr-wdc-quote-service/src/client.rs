// ----- standard library imports
use std::{collections::HashMap, ops::Deref};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    client::{
        admin::notification::Client as NotificationClient,
        core::Client as CoreClient,
        ebill::Client as EBillClient,
        treasury::{Client as TreasuryClient, Error as TreasuryError},
    },
    core::BillId,
    wire::{bill as wire_bill, notification as wire_notification, quotes as wire_quotes},
};
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::{
    error::{Error, Result},
    service::{MintingStatus, WdcClient},
};

// ----- end imports

#[derive(Debug, Clone)]
pub struct WildcatCl {
    pub core: CoreClient,
    pub treasury: TreasuryClient,
    pub ebill: EBillClient,
    pub notification: NotificationClient,
}

#[async_trait]
impl WdcClient for WildcatCl {
    async fn get_keyset_with_expiration_date(
        &self,
        redemption_date: time::Date,
    ) -> Result<cashu::Id> {
        let kinfo = self
            .core
            .get_or_create_keyset_with_expiration(redemption_date)
            .await?;
        Ok(kinfo.id.into())
    }

    async fn add_new_mint_operation(
        &self,
        qid: Uuid,
        kid: cashu::Id,
        c_pk: cashu::PublicKey,
        amount: cashu::Amount,
        discounted: cashu::Amount,
        bill_id: BillId,
    ) -> Result<()> {
        self.treasury
            .new_ebill_mint_operation(qid, kid.into(), *c_pk.deref(), amount, discounted, bill_id)
            .await?;
        Ok(())
    }

    async fn get_minting_status(&self, qid: Uuid) -> Result<MintingStatus> {
        let response = self.treasury.ebill_mint_operation_status(qid).await;
        match response {
            Ok(status) => Ok(MintingStatus::Enabled(status.current)),
            Err(TreasuryError::ResourceNotFound(_)) => Ok(MintingStatus::Disabled),
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

    async fn validate_endorsed_bill_matches_shared_bill(
        &self,
        bill_id: BillId,
        shared_bill_data: String,
    ) -> Result<bool> {
        self.ebill
            .validate_endorsed_bill_matches_shared_bill(bill_id, shared_bill_data)
            .await?;
        Ok(true)
    }

    async fn get_shared_ebill_history(
        &self,
        bill_id: BillId,
        shared_bill_data: String,
    ) -> Result<Vec<wire_bill::BillHistoryBlock>> {
        let ebill = self
            .ebill
            .get_shared_bill_history(bill_id, shared_bill_data)
            .await?;
        Ok(ebill)
    }

    async fn get_ebill(&self, bid: BillId) -> Result<wire_bill::BitcreditBill> {
        let ebill = self.ebill.get_bill(&bid).await?;
        Ok(ebill)
    }

    async fn notify_new_ebill_quote(
        &self,
        qid: Uuid,
        sum: bitcoin::Amount,
        drawee: String,
        maturity_date: time::Date,
    ) -> Result<()> {
        let fields = HashMap::from([
            (String::from("quote_id"), qid.to_string()),
            (String::from("amount"), sum.to_sat().to_string()),
            (String::from("drawee"), drawee),
            (String::from("maturity_date"), maturity_date.to_string()),
        ]);
        let request = wire_notification::NotificationRequest::NewEbillQuote { fields };
        self.notification.notify(request).await?;
        Ok(())
    }
}
