// ----- standard library imports
// ----- extra library imports
use bcr_common::{cashu, client::admin::core::BRError, core::BillId, wire::mint as wire_mint};
use uuid::Uuid;
// ----- local imports
use crate::{
    ebill::{ClowderClient, MintOperation, Repository, WildcatClient},
    error::{Error, Result},
    TStamp,
};

// ----- end imports

fn is_same_op(a: &MintOperation, b: &MintOperation) -> bool {
    a.uid == b.uid
        && a.kid == b.kid
        && a.pub_key == b.pub_key
        && a.target == b.target
        && a.bill_id == b.bill_id
}

pub struct Service {
    pub repo: Box<dyn Repository>,
    pub clowdercl: Box<dyn ClowderClient>,
    pub multiplier: cashu::Amount,
}

impl Service {
    #[allow(clippy::too_many_arguments)]
    pub async fn new_minting_operation(
        &self,
        wdc_cl: &dyn WildcatClient,
        uid: Uuid,
        kid: cashu::Id,
        pub_key: cashu::PublicKey,
        amount: cashu::Amount,
        bill_id: BillId,
        _now: TStamp,
    ) -> Result<()> {
        let _kinfo = wdc_cl.info(kid).await?;
        let new = MintOperation {
            uid,
            kid,
            pub_key,
            target: amount,
            minted: cashu::Amount::ZERO,
            bill_id,
        };
        if let Some(existing) = self.repo.mint_lookup_by_bill(new.bill_id.clone()).await? {
            if !is_same_op(&existing, &new) {
                let msg = format!(
                    "bill {} already bound to mint operation {}",
                    new.bill_id, existing.uid
                );
                return Err(Error::Conflict(msg));
            }
        } else if let Err(e) = self.repo.mint_store(new.clone()).await {
            match e {
                Error::Conflict(_) => {
                    match self.repo.mint_lookup_by_bill(new.bill_id.clone()).await? {
                        Some(existing) if is_same_op(&existing, &new) => {}
                        _ => return Err(e),
                    }
                }
                _ => return Err(e),
            }
        }
        self.clowdercl
            .register_ebill(new.bill_id.clone(), amount)
            .await
    }

    pub async fn mintop_status(&self, uid: Uuid) -> Result<MintOperation> {
        let operation = self.repo.mint_load(uid).await?;
        Ok(operation)
    }

    pub async fn list_mintops_for_kid(&self, kid: cashu::Id) -> Result<Vec<MintOperation>> {
        self.repo.mint_list(kid).await
    }

    pub async fn mint(
        &self,
        wdc_cl: &dyn WildcatClient,
        request: wire_mint::EbillMintRequest,
    ) -> Result<wire_mint::EbillMintResponse> {
        // basic checks
        bcr_wdc_utils::signatures::basic_blinds_checks(&request.outputs)
            .map_err(|e| Error::InvalidInput(BRError::Generic(e.to_string())))?;
        let output_amount = request
            .outputs
            .iter()
            .fold(cashu::Amount::ZERO, |acc, blind| {
                acc.checked_add(blind.amount)
                    .expect("basic_blinds_checks should prevent overflow")
            });
        let operation = self.repo.mint_load(request.quote).await?;
        let signature_is_ok = request.verify_signature(&operation.pub_key);
        if !signature_is_ok {
            return Err(Error::InvalidInput(BRError::Generic(String::from(
                "invalid signature",
            ))));
        }
        let same_kid = request
            .outputs
            .iter()
            .all(|blind| blind.keyset_id == operation.kid);
        if !same_kid {
            return Err(Error::InvalidInput(BRError::Generic(String::from(
                "invalid keyset id",
            ))));
        }
        // every output must have a key in the keyset, or signing fails after the allowance is
        // already reserved
        let kinfo = wdc_cl.info(operation.kid).await?;
        let unsupported = request
            .outputs
            .iter()
            .find(|blind| !kinfo.amounts.contains(&blind.amount.to_u64()));
        if let Some(blind) = unsupported {
            let msg = format!("unsupported amount {}", blind.amount);
            return Err(Error::InvalidInput(BRError::Generic(msg)));
        }
        let target = operation
            .target
            .checked_mul(self.multiplier)
            .unwrap_or(operation.target);
        if operation.minted + output_amount > target {
            return Err(Error::InvalidInput(BRError::Generic(String::from(
                "exceeding amount",
            ))));
        }
        let uid = operation.uid;
        let old = operation.minted;
        let new = operation.minted + output_amount;
        self.repo.mint_update_field(uid, old, new).await?;
        let signatures = match wdc_cl.sign(&request.outputs).await {
            Ok(signatures) => signatures,
            Err(e) => {
                self.repo.mint_update_field(uid, new, old).await?;
                return Err(e);
            }
        };
        let signatures = match self
            .clowdercl
            .minting_ebill(
                operation.kid,
                request.quote,
                output_amount,
                operation.bill_id.clone(),
                signatures.clone(),
            )
            .await
        {
            Ok(signatures) => signatures,
            Err(e) => {
                self.repo.mint_update_field(uid, new, old).await?;
                return Err(e);
            }
        };
        let store_signatures = request
            .outputs
            .iter()
            .map(|b| b.blinded_secret)
            .zip(signatures.iter().cloned())
            .collect();
        match wdc_cl.store(store_signatures).await {
            Ok(_) => {}
            Err(e) => {
                self.repo.mint_update_field(uid, new, old).await?;
                return Err(e);
            }
        }
        Ok(wire_mint::EbillMintResponse { signatures })
    }

    pub async fn request_to_pay_ebill(
        &self,
        wdc_cl: &dyn WildcatClient,
        bid: BillId,
        amount: bitcoin::Amount,
        deadline: TStamp,
    ) -> Result<()> {
        let (block_id, previous_block_hash) = wdc_cl.prepare_request_to_pay(bid.clone()).await?;
        let payment_address = self
            .clowdercl
            .request_onchain_ebill_address(bid.clone(), block_id, previous_block_hash)
            .await?;
        let _bill_private_key = wdc_cl
            .request_to_pay(bid.clone(), deadline, payment_address.clone())
            .await?;
        self.clowdercl
            .request_to_pay_ebill(bid, payment_address, block_id, previous_block_hash, amount)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ebill::{MockClowderClient, MockRepository, MockWildcatClient};
    use bcr_common::{cashu, client::clowder::ClowderClientError, core_tests};
    use bcr_wdc_utils::signatures::test_utils as signatures_test;
    use mockall::predicate::eq;

    #[tokio::test]
    async fn new_minting_operation_missing_keyset() {
        let repo = MockRepository::new();
        let clowder_cl = MockClowderClient::new();
        let mut core_cl = MockWildcatClient::new();
        let kid = bcr_common::core_tests::generate_random_ecash_keyset().0.id;
        let uid = Uuid::new_v4();
        let pub_key = bcr_common::core::generate_random_keypair()
            .public_key()
            .into();
        let amount = cashu::Amount::from(32);
        let bill_id = core_tests::random_bill_id();
        core_cl
            .expect_info()
            .times(1)
            .with(eq(cashu::Id::from(kid)))
            .returning(|_| Err(Error::InvalidInput(BRError::Generic(String::new()))));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            repo: Box::new(repo),
            multiplier: cashu::Amount::ONE,
        };
        let now = time::OffsetDateTime::now_utc();
        let err = service
            .new_minting_operation(&core_cl, uid, kid.into(), pub_key, amount, bill_id, now)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[tokio::test]
    async fn new_minting_operation_ok() {
        let mut repo = MockRepository::new();
        let mut clowder_cl = MockClowderClient::new();
        let mut core_cl = MockWildcatClient::new();
        let (kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let uid = Uuid::new_v4();
        let pub_key = bcr_common::core::generate_random_keypair()
            .public_key()
            .into();
        let amount = cashu::Amount::from(64);
        let bill_id = core_tests::random_bill_id();
        core_cl
            .expect_info()
            .times(1)
            .with(eq(cashu::Id::from(kid)))
            .returning(move |_| Ok(kinfo.clone()));
        repo.expect_mint_lookup_by_bill()
            .times(1)
            .with(eq(bill_id.clone()))
            .returning(|_| Ok(None));
        clowder_cl
            .expect_register_ebill()
            .times(1)
            .with(eq(bill_id.clone()), eq(amount))
            .returning(|_, _| Ok(()));
        let mintop = MintOperation {
            uid,
            kid: kid.into(),
            pub_key,
            target: amount,
            minted: cashu::Amount::ZERO,
            bill_id: bill_id.clone(),
        };
        repo.expect_mint_store()
            .times(1)
            .with(eq(mintop))
            .returning(|_| Ok(()));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            repo: Box::new(repo),
            multiplier: cashu::Amount::ONE,
        };
        let now = time::OffsetDateTime::now_utc();
        service
            .new_minting_operation(&core_cl, uid, kid.into(), pub_key, amount, bill_id, now)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn new_minting_operation_refire_same_uid_is_idempotent() {
        let mut repo = MockRepository::new();
        let mut clowder_cl = MockClowderClient::new();
        let mut core_cl = MockWildcatClient::new();
        let (kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let uid = Uuid::new_v4();
        let pub_key = bcr_common::core::generate_random_keypair()
            .public_key()
            .into();
        let amount = cashu::Amount::from(64);
        let bill_id = core_tests::random_bill_id();
        core_cl
            .expect_info()
            .times(1)
            .with(eq(cashu::Id::from(kid)))
            .returning(move |_| Ok(kinfo.clone()));
        let existing = MintOperation {
            uid,
            kid: kid.into(),
            pub_key,
            target: amount,
            minted: cashu::Amount::ZERO,
            bill_id: bill_id.clone(),
        };
        repo.expect_mint_lookup_by_bill()
            .times(1)
            .with(eq(bill_id.clone()))
            .returning(move |_| Ok(Some(existing.clone())));
        clowder_cl
            .expect_register_ebill()
            .times(1)
            .with(eq(bill_id.clone()), eq(amount))
            .returning(|_, _| Ok(()));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            repo: Box::new(repo),
            multiplier: cashu::Amount::ONE,
        };
        let now = time::OffsetDateTime::now_utc();
        service
            .new_minting_operation(&core_cl, uid, kid.into(), pub_key, amount, bill_id, now)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn new_minting_operation_other_op_same_bill_rejected() {
        let mut repo = MockRepository::new();
        let clowder_cl = MockClowderClient::new();
        let mut core_cl = MockWildcatClient::new();
        let (kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let pub_key = bcr_common::core::generate_random_keypair()
            .public_key()
            .into();
        let amount = cashu::Amount::from(64);
        let bill_id = core_tests::random_bill_id();
        core_cl
            .expect_info()
            .times(1)
            .with(eq(cashu::Id::from(kid)))
            .returning(move |_| Ok(kinfo.clone()));
        let existing = MintOperation {
            uid: Uuid::new_v4(),
            kid: kid.into(),
            pub_key,
            target: amount,
            minted: cashu::Amount::ZERO,
            bill_id: bill_id.clone(),
        };
        repo.expect_mint_lookup_by_bill()
            .times(1)
            .with(eq(bill_id.clone()))
            .returning(move |_| Ok(Some(existing.clone())));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            repo: Box::new(repo),
            multiplier: cashu::Amount::ONE,
        };
        let now = time::OffsetDateTime::now_utc();
        let err = service
            .new_minting_operation(
                &core_cl,
                Uuid::new_v4(),
                kid.into(),
                pub_key,
                amount,
                bill_id,
                now,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Conflict(_)));
    }

    #[tokio::test]
    async fn new_minting_operation_refire_same_uid_different_params_rejected() {
        let mut repo = MockRepository::new();
        let clowder_cl = MockClowderClient::new();
        let mut core_cl = MockWildcatClient::new();
        let (kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let uid = Uuid::new_v4();
        let pub_key = bcr_common::core::generate_random_keypair()
            .public_key()
            .into();
        let amount = cashu::Amount::from(64);
        let bill_id = core_tests::random_bill_id();
        core_cl
            .expect_info()
            .times(1)
            .with(eq(cashu::Id::from(kid)))
            .returning(move |_| Ok(kinfo.clone()));
        let existing = MintOperation {
            uid,
            kid: kid.into(),
            pub_key,
            target: cashu::Amount::from(128),
            minted: cashu::Amount::ZERO,
            bill_id: bill_id.clone(),
        };
        repo.expect_mint_lookup_by_bill()
            .times(1)
            .with(eq(bill_id.clone()))
            .returning(move |_| Ok(Some(existing.clone())));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            repo: Box::new(repo),
            multiplier: cashu::Amount::ONE,
        };
        let now = time::OffsetDateTime::now_utc();
        let err = service
            .new_minting_operation(&core_cl, uid, kid.into(), pub_key, amount, bill_id, now)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Conflict(_)));
    }

    #[tokio::test]
    async fn new_minting_operation_register_failure_heals_on_refire() {
        let mut repo = MockRepository::new();
        let mut clowder_cl = MockClowderClient::new();
        let mut core_cl = MockWildcatClient::new();
        let (kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let uid = Uuid::new_v4();
        let pub_key = bcr_common::core::generate_random_keypair()
            .public_key()
            .into();
        let amount = cashu::Amount::from(64);
        let bill_id = core_tests::random_bill_id();
        core_cl
            .expect_info()
            .times(2)
            .with(eq(cashu::Id::from(kid)))
            .returning(move |_| Ok(kinfo.clone()));
        repo.expect_mint_lookup_by_bill()
            .times(1)
            .with(eq(bill_id.clone()))
            .returning(|_| Ok(None));
        let stored = MintOperation {
            uid,
            kid: kid.into(),
            pub_key,
            target: amount,
            minted: cashu::Amount::ZERO,
            bill_id: bill_id.clone(),
        };
        repo.expect_mint_store()
            .times(1)
            .with(eq(stored.clone()))
            .returning(|_| Ok(()));
        clowder_cl
            .expect_register_ebill()
            .times(1)
            .with(eq(bill_id.clone()), eq(amount))
            .returning(|_, _| Err(clowder_transport_error()));
        repo.expect_mint_lookup_by_bill()
            .times(1)
            .with(eq(bill_id.clone()))
            .returning(move |_| Ok(Some(stored.clone())));
        clowder_cl
            .expect_register_ebill()
            .times(1)
            .with(eq(bill_id.clone()), eq(amount))
            .returning(|_, _| Ok(()));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            repo: Box::new(repo),
            multiplier: cashu::Amount::ONE,
        };
        let now = time::OffsetDateTime::now_utc();
        let err = service
            .new_minting_operation(
                &core_cl,
                uid,
                kid.into(),
                pub_key,
                amount,
                bill_id.clone(),
                now,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, Error::ClowderClient(_)));
        service
            .new_minting_operation(&core_cl, uid, kid.into(), pub_key, amount, bill_id, now)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn new_minting_operation_store_race_is_idempotent() {
        let mut repo = MockRepository::new();
        let mut clowder_cl = MockClowderClient::new();
        let mut core_cl = MockWildcatClient::new();
        let (kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let uid = Uuid::new_v4();
        let pub_key = bcr_common::core::generate_random_keypair()
            .public_key()
            .into();
        let amount = cashu::Amount::from(64);
        let bill_id = core_tests::random_bill_id();
        core_cl
            .expect_info()
            .times(1)
            .with(eq(cashu::Id::from(kid)))
            .returning(move |_| Ok(kinfo.clone()));
        repo.expect_mint_lookup_by_bill()
            .times(1)
            .with(eq(bill_id.clone()))
            .returning(|_| Ok(None));
        clowder_cl
            .expect_register_ebill()
            .times(1)
            .with(eq(bill_id.clone()), eq(amount))
            .returning(|_, _| Ok(()));
        repo.expect_mint_store()
            .times(1)
            .returning(|op| Err(Error::Conflict(op.uid.to_string())));
        let stored = MintOperation {
            uid,
            kid: kid.into(),
            pub_key,
            target: amount,
            minted: cashu::Amount::ZERO,
            bill_id: bill_id.clone(),
        };
        repo.expect_mint_lookup_by_bill()
            .times(1)
            .with(eq(bill_id.clone()))
            .returning(move |_| Ok(Some(stored.clone())));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            repo: Box::new(repo),
            multiplier: cashu::Amount::ONE,
        };
        let now = time::OffsetDateTime::now_utc();
        service
            .new_minting_operation(&core_cl, uid, kid.into(), pub_key, amount, bill_id, now)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn new_minting_operation_store_race_other_op_rejected() {
        let mut repo = MockRepository::new();
        let clowder_cl = MockClowderClient::new();
        let mut core_cl = MockWildcatClient::new();
        let (kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let uid = Uuid::new_v4();
        let pub_key = bcr_common::core::generate_random_keypair()
            .public_key()
            .into();
        let amount = cashu::Amount::from(64);
        let bill_id = core_tests::random_bill_id();
        core_cl
            .expect_info()
            .times(1)
            .with(eq(cashu::Id::from(kid)))
            .returning(move |_| Ok(kinfo.clone()));
        repo.expect_mint_lookup_by_bill()
            .times(1)
            .with(eq(bill_id.clone()))
            .returning(|_| Ok(None));
        repo.expect_mint_store()
            .times(1)
            .returning(|op| Err(Error::Conflict(op.uid.to_string())));
        let stored = MintOperation {
            uid: Uuid::new_v4(),
            kid: kid.into(),
            pub_key,
            target: amount,
            minted: cashu::Amount::ZERO,
            bill_id: bill_id.clone(),
        };
        repo.expect_mint_lookup_by_bill()
            .times(1)
            .with(eq(bill_id.clone()))
            .returning(move |_| Ok(Some(stored.clone())));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            repo: Box::new(repo),
            multiplier: cashu::Amount::ONE,
        };
        let now = time::OffsetDateTime::now_utc();
        let err = service
            .new_minting_operation(&core_cl, uid, kid.into(), pub_key, amount, bill_id, now)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Conflict(_)));
    }

    #[tokio::test]
    async fn mint_ok() {
        let mut mintop_repo = MockRepository::new();
        let mut clowder_cl = MockClowderClient::new();
        let mut core_cl = MockWildcatClient::new();
        let (kinfo, keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let uid = Uuid::new_v4();
        let kp = bcr_common::core::generate_random_keypair();
        let pub_key = cashu::PublicKey::from(kp.public_key());
        let amounts = [cashu::Amount::from(128), cashu::Amount::from(64)];
        let total = amounts
            .iter()
            .fold(cashu::Amount::ZERO, |acc, amount| acc + *amount);
        let bill_id = core_tests::random_bill_id();
        mintop_repo
            .expect_mint_update_field()
            .times(1)
            .with(eq(uid), eq(cashu::Amount::ZERO), eq(total))
            .returning(|_, _, _| Ok(()));
        let info = kinfo.clone();
        core_cl
            .expect_info()
            .times(1)
            .with(eq(cashu::Id::from(kid)))
            .returning(move |_| Ok(info.clone()));
        core_cl.expect_sign().times(1).returning(move |msgs| {
            let amounts: Vec<cashu::Amount> = msgs.iter().map(|msg| msg.amount).collect();
            let signs = core_tests::generate_ecash_signatures(&keyset, &amounts);
            Ok(signs)
        });
        let mintop = MintOperation {
            uid,
            kid: kid.into(),
            pub_key,
            target: total,
            minted: cashu::Amount::ZERO,
            bill_id: bill_id.clone(),
        };
        mintop_repo
            .expect_mint_load()
            .times(1)
            .with(eq(uid))
            .returning(move |_| Ok(mintop.clone()));
        clowder_cl
            .expect_minting_ebill()
            .times(1)
            .returning(|_, _, _, _, signatures| Ok(signatures));
        core_cl.expect_store().times(1).returning(|_| Ok(()));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            repo: Box::new(mintop_repo),
            multiplier: cashu::Amount::ONE,
        };
        let outputs = signatures_test::generate_blinds(kid.into(), &amounts);
        let blinds = outputs.iter().map(|(blind, _, _)| blind.clone()).collect();
        let request = wire_mint::EbillMintRequest::new(uid, blinds, &kp);
        let response = service.mint(&core_cl, request).await.unwrap();
        assert_eq!(response.signatures.len(), amounts.len());
    }

    #[tokio::test]
    async fn mint_missing_mintop() {
        let mut mintop_repo = MockRepository::new();
        let clowder_cl = MockClowderClient::new();
        let core_cl = MockWildcatClient::new();
        let kid = bcr_common::core_tests::generate_random_ecash_keyset().0.id;
        let uid = Uuid::new_v4();
        let kp = bcr_common::core::generate_random_keypair();
        let amounts = [cashu::Amount::from(128)];
        mintop_repo
            .expect_mint_load()
            .times(1)
            .with(eq(uid))
            .returning(move |_| Err(Error::InvalidInput(BRError::Generic(String::new()))));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            repo: Box::new(mintop_repo),
            multiplier: cashu::Amount::ONE,
        };
        let outputs = signatures_test::generate_blinds(kid.into(), &amounts);
        let blinds = outputs.iter().map(|(blind, _, _)| blind.clone()).collect();
        let request = wire_mint::EbillMintRequest::new(uid, blinds, &kp);
        let err = service.mint(&core_cl, request).await.unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[tokio::test]
    async fn mint_unsupported_amount_does_not_reserve() {
        let mintop_repo = MockRepository::new();
        let clowder_cl = MockClowderClient::new();
        let core_cl = MockWildcatClient::new();
        let (kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let uid = Uuid::new_v4();
        let kp = bcr_common::core::generate_random_keypair();
        let amounts = [cashu::Amount::from(3)];
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            repo: Box::new(mintop_repo),
            multiplier: cashu::Amount::ONE,
        };
        let outputs = signatures_test::generate_blinds(kid.into(), &amounts);
        let blinds = outputs.iter().map(|(blind, _, _)| blind.clone()).collect();
        let request = wire_mint::EbillMintRequest::new(uid, blinds, &kp);
        let err = service.mint(&core_cl, request).await.unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[tokio::test]
    async fn mint_sign_failure_releases_reservation() {
        let mut mintop_repo = MockRepository::new();
        let clowder_cl = MockClowderClient::new();
        let mut core_cl = MockWildcatClient::new();
        let (kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let uid = Uuid::new_v4();
        let kp = bcr_common::core::generate_random_keypair();
        let amounts = [cashu::Amount::from(1), cashu::Amount::from(2)];
        let total = cashu::Amount::from(3);
        let mintop = MintOperation {
            uid,
            kid: kid.into(),
            pub_key: cashu::PublicKey::from(kp.public_key()),
            target: total,
            minted: cashu::Amount::ZERO,
            bill_id: core_tests::random_bill_id(),
        };
        mintop_repo
            .expect_mint_load()
            .times(1)
            .with(eq(uid))
            .returning(move |_| Ok(mintop.clone()));
        mintop_repo
            .expect_mint_update_field()
            .times(1)
            .with(eq(uid), eq(cashu::Amount::ZERO), eq(total))
            .returning(|_, _, _| Ok(()));
        mintop_repo
            .expect_mint_update_field()
            .times(1)
            .with(eq(uid), eq(total), eq(cashu::Amount::ZERO))
            .returning(|_, _, _| Ok(()));
        core_cl
            .expect_info()
            .times(1)
            .with(eq(cashu::Id::from(kid)))
            .returning(move |_| Ok(kinfo.clone()));
        core_cl
            .expect_sign()
            .times(1)
            .returning(|_| Err(Error::Internal(String::from("sign failed"))));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            repo: Box::new(mintop_repo),
            multiplier: cashu::Amount::ONE,
        };
        let outputs = signatures_test::generate_blinds(kid.into(), &amounts);
        let blinds = outputs.iter().map(|(blind, _, _)| blind.clone()).collect();
        let request = wire_mint::EbillMintRequest::new(uid, blinds, &kp);
        let err = service.mint(&core_cl, request).await.unwrap_err();
        assert!(matches!(err, Error::Internal(_)));
    }

    fn clowder_transport_error() -> Error {
        Error::ClowderClient(
            bcr_common::client::clowder::ClowderClientError::Serialization(String::from("timeout")),
        )
    }

    fn mint_clowder_failure_service(
        clowder_cl: MockClowderClient,
    ) -> (Service, MockWildcatClient, wire_mint::EbillMintRequest) {
        let mut mintop_repo = MockRepository::new();
        let mut core_cl = MockWildcatClient::new();
        let (kinfo, keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let uid = Uuid::new_v4();
        let kp = bcr_common::core::generate_random_keypair();
        let pub_key = cashu::PublicKey::from(kp.public_key());
        let amounts = [cashu::Amount::from(128)];
        let total = amounts
            .iter()
            .fold(cashu::Amount::ZERO, |acc, amount| acc + *amount);
        let bill_id = core_tests::random_bill_id();
        mintop_repo
            .expect_mint_update_field()
            .times(1)
            .with(eq(uid), eq(cashu::Amount::ZERO), eq(total))
            .returning(|_, _, _| Ok(()));
        mintop_repo
            .expect_mint_update_field()
            .times(1)
            .with(eq(uid), eq(total), eq(cashu::Amount::ZERO))
            .returning(|_, _, _| Ok(()));
        let info = kinfo.clone();
        core_cl
            .expect_info()
            .times(1)
            .with(eq(cashu::Id::from(kid)))
            .returning(move |_| Ok(info.clone()));
        core_cl.expect_sign().times(1).returning(move |msgs| {
            let amounts: Vec<cashu::Amount> = msgs.iter().map(|msg| msg.amount).collect();
            let signs = core_tests::generate_ecash_signatures(&keyset, &amounts);
            Ok(signs)
        });
        let mintop = MintOperation {
            uid,
            kid: kid.into(),
            pub_key,
            target: total,
            minted: cashu::Amount::ZERO,
            bill_id,
        };
        mintop_repo
            .expect_mint_load()
            .times(1)
            .with(eq(uid))
            .returning(move |_| Ok(mintop.clone()));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            repo: Box::new(mintop_repo),
            multiplier: cashu::Amount::ONE,
        };
        let outputs = signatures_test::generate_blinds(kid.into(), &amounts);
        let blinds = outputs.iter().map(|(blind, _, _)| blind.clone()).collect();
        let request = wire_mint::EbillMintRequest::new(uid, blinds, &kp);
        (service, core_cl, request)
    }

    #[tokio::test]
    async fn mint_clowder_error_releases_reservation() {
        let mut clowder_cl = MockClowderClient::new();
        clowder_cl
            .expect_minting_ebill()
            .times(1)
            .returning(|_, _, _, _, _| Err(clowder_transport_error()));
        let (service, core_cl, request) = mint_clowder_failure_service(clowder_cl);
        let err = service.mint(&core_cl, request).await.unwrap_err();
        assert!(matches!(err, Error::ClowderClient(_)));
    }

    #[tokio::test]
    async fn mint_clowder_rejection_releases_reservation() {
        let mut clowder_cl = MockClowderClient::new();
        clowder_cl
            .expect_minting_ebill()
            .times(1)
            .returning(|_, _, _, _, _| {
                Err(Error::ClowderClient(ClowderClientError::Rejected(
                    bcr_common::wire::clowder::ClowderRejection::Expired,
                )))
            });
        let (service, core_cl, request) = mint_clowder_failure_service(clowder_cl);
        let err = service.mint(&core_cl, request).await.unwrap_err();
        assert!(matches!(
            err,
            Error::ClowderClient(ClowderClientError::Rejected(_))
        ));
    }
}
