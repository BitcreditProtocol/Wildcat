// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    client::{
        core::{Client as CoreClient, Error as CoreError},
        ebill::Client as EbillClient,
    },
    core::BillId,
    wire::quotes as wire_quotes,
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
    pub ebill: EbillClient,
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
        self.core
            .new_mint_operation(qid, kid, pk, target, bill_id)
            .await?;
        Ok(())
    }
    async fn sign(&self, msg: &cashu::BlindedMessage) -> Result<cashu::BlindSignature> {
        let signatures = self.core.sign(msg).await?;
        Ok(signatures)
    }
    async fn get_minting_status(&self, qid: Uuid) -> Result<MintingStatus> {
        let response = self.core.mint_operation_status(qid).await;
        match response {
            Ok(status) => Ok(MintingStatus::Enabled(status.current)),
            Err(CoreError::MintOpNotFound(_)) => Ok(MintingStatus::Disabled),
            Err(e) => Err(Error::CoreHandler(e)),
        }
    }
    async fn validate_and_decrypt_shared_bill(
        &self,
        shared_bill: &wire_quotes::SharedBill,
    ) -> Result<wire_quotes::BillInfo> {
        self.ebill
            .validate_and_decrypt_shared_bill(shared_bill)
            .await
            .map_err(Error::EbillClient)
    }
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;
    use crate::TStamp;
    use bcr_wdc_utils::keys::KeysetEntry;
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    #[derive(Clone, Debug, Default)]
    pub struct DummyKeysHandler {
        keys: Arc<Mutex<HashMap<cashu::Id, KeysetEntry>>>,
    }

    #[async_trait]
    impl WdcClient for DummyKeysHandler {
        async fn get_keyset_with_redemption_date(
            &self,
            redemption_date: chrono::NaiveDate,
        ) -> Result<cashu::Id> {
            let mut locked = self.keys.lock().unwrap();
            for (kid, (info, _)) in locked.iter() {
                let key_expiry = info.final_expiry.unwrap_or_default() as i64;
                let exp = TStamp::from_timestamp(key_expiry, 0).unwrap_or_default();
                if exp.date_naive() == redemption_date {
                    return Ok(*kid);
                }
            }
            let mut keysentry = bcr_wdc_utils::keys::test_utils::generate_keyset();
            let kid = keysentry.0.id;
            keysentry.0.final_expiry = Some(
                redemption_date
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc()
                    .timestamp() as u64,
            );
            keysentry.1.final_expiry = keysentry.0.final_expiry;
            locked.insert(kid, keysentry.clone());
            Ok(kid)
        }
        async fn add_new_mint_operation(
            &self,
            _qid: Uuid,
            _kid: cashu::Id,
            _pk: cashu::PublicKey,
            _target: cashu::Amount,
            _bill_id: bcr_common::core::BillId,
        ) -> Result<()> {
            Ok(())
        }
        async fn sign(&self, msg: &cashu::BlindedMessage) -> Result<cashu::BlindSignature> {
            let locked = self.keys.lock().unwrap();
            let (_, keyset) = locked.get(&msg.keyset_id).expect("Keyset not found");
            let sig = bcr_wdc_utils::keys::sign_with_keys(keyset, msg).expect("sign_with_keys");
            Ok(sig)
        }
    }
}
