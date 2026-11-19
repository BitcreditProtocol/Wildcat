// ----- standard library imports
// ----- extra library imports
use bcr_common::{
    cashu, client::clowder::ClowderClientError, core::BillId, ecash, wire::mint as wire_mint,
};
use uuid::Uuid;
// ----- local imports
use crate::{
    ebill::{ClowderClient, MintOperation, Repository, VaultService, WildcatClient},
    error::{Error, Result},
    TStamp,
};

// ----- end imports

const MINTING_EBILL_ATTEMPTS: u32 = 2;
const MINTING_EBILL_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

fn is_same_op(a: &MintOperation, b: &MintOperation) -> bool {
    a.uid == b.uid
        && a.kid == b.kid
        && a.pub_key == b.pub_key
        && a.target == b.target
        && a.fee == b.fee
        && a.bill_id == b.bill_id
}

pub struct Service {
    pub repo: Box<dyn Repository>,
    pub wildcatcl: Box<dyn WildcatClient>,
    pub clowdercl: Box<dyn ClowderClient>,
    pub multiplier: cashu::Amount,
}

impl Service {
    /// amount is the bill sum, discounted is what the wallet can mint,
    /// the difference is the mint fee, minted by the treasury with the first wallet mint
    #[allow(clippy::too_many_arguments)]
    pub async fn new_minting_operation(
        &self,
        uid: Uuid,
        kid: cashu::Id,
        pub_key: cashu::PublicKey,
        amount: cashu::Amount,
        discounted: cashu::Amount,
        bill_id: BillId,
        _now: TStamp,
    ) -> Result<()> {
        let _kinfo = self.wildcatcl.info(kid).await?;
        let fee = amount.checked_sub(discounted).ok_or_else(|| {
            Error::InvalidInput(format!(
                "discounted {discounted} exceeds bill amount {amount}"
            ))
        })?;
        let new = MintOperation {
            uid,
            kid,
            pub_key,
            target: discounted,
            minted: cashu::Amount::ZERO,
            bill_id,
            fee,
        };
        let stored = self.repo.mint_store(new.clone()).await;
        let op = match stored {
            Ok(()) => new,
            Err(Error::AlreadyExists(_)) => {
                let existing = self.repo.mint_lookup_by_bill(new.bill_id.clone()).await?;
                match existing {
                    Some(old) => {
                        let same = is_same_op(&old, &new);
                        if !same {
                            let msg = format!(
                                "bill {} already bound to mint operation {}",
                                new.bill_id, old.uid
                            );
                            return Err(Error::AlreadyExists(msg));
                        }
                        old
                    }
                    None => {
                        let msg = format!(
                            "mint operation {} already exists but not found by bill {}",
                            new.uid, new.bill_id
                        );
                        return Err(Error::AlreadyExists(msg));
                    }
                }
            }
            Err(e) => return Err(e),
        };
        self.clowdercl.register_ebill(op.bill_id, amount).await
    }

    /// signs the fee ecash and unblinds it, returns the signatures for clowder and the proofs for the vault
    async fn sign_fee(
        &self,
        kid: cashu::Id,
        fee: cashu::Amount,
    ) -> Result<(Vec<cashu::BlindSignature>, Vec<cashu::Proof>)> {
        let keyset = self.wildcatcl.keys(kid).await?;
        let premint = cashu::PreMintSecrets::random(
            kid,
            fee,
            &cashu::amount::SplitTarget::None,
            &bcr_wdc_utils::keys::to_fee_and_amounts(&keyset),
        )?;
        let signatures = self.wildcatcl.sign(&premint.blinded_messages()).await?;
        let (rs, secrets) = premint
            .secrets
            .into_iter()
            .map(|pre| (pre.r, pre.secret))
            .unzip();
        let keys = cashu::KeySet::from(keyset);
        let proofs = cashu::dhke::construct_proofs(signatures.clone(), rs, secrets, &keys.keys)?;
        Ok((signatures, proofs))
    }

    async fn report_minting(
        &self,
        kid: cashu::Id,
        qid: Uuid,
        amount: cashu::Amount,
        bill_id: BillId,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        let mut attempts = 1;
        loop {
            let response = self
                .clowdercl
                .minting_ebill(kid, qid, amount, bill_id.clone(), signatures.clone())
                .await;
            match response {
                Ok(signatures) => return Ok(signatures),
                Err(e @ Error::ClowderNatsClient(_))
                    if attempts < MINTING_EBILL_ATTEMPTS
                        && !matches!(
                            e,
                            Error::ClowderNatsClient(ClowderClientError::Rejected(_))
                        ) =>
                {
                    tracing::warn!("minting_ebill attempt {attempts} failed, retrying: {e}");
                    attempts += 1;
                    tokio::time::sleep(MINTING_EBILL_RETRY_DELAY).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub async fn mintop_status(&self, uid: Uuid) -> Result<MintOperation> {
        let operation = self.repo.mint_load(uid).await?;
        Ok(operation)
    }

    pub async fn list_mintops_for_kid(&self, kid: cashu::Id) -> Result<Vec<MintOperation>> {
        self.repo.mint_list(kid).await
    }

    /// vault receives the mint fee proofs when the request carries the fee
    pub async fn mint(
        &self,
        request: wire_mint::EbillMintRequest,
        vault: &dyn VaultService,
    ) -> Result<wire_mint::EbillMintResponse> {
        // basic checks
        let blinds: Vec<ecash::BlindedMessage> =
            request.outputs.iter().cloned().map(From::from).collect();
        bcr_wdc_utils::signatures::basic_blinds_checks(&blinds)
            .map_err(|e| Error::InvalidInput(e.to_string()))?;
        let output_amount = request
            .outputs
            .iter()
            .fold(cashu::Amount::ZERO, |acc, blind| acc + blind.amount);
        // minted must strictly grow, so that only one request moves it away from zero
        // and carries the mint fee
        if output_amount == cashu::Amount::ZERO {
            return Err(Error::InvalidInput(String::from("empty outputs")));
        }
        let operation = self.repo.mint_load(request.quote).await?;
        let signature_is_ok = request.verify_signature(&operation.pub_key);
        if !signature_is_ok {
            return Err(Error::InvalidInput(String::from("invalid signature")));
        }
        let same_kid = request
            .outputs
            .iter()
            .all(|blind| blind.keyset_id == operation.kid);
        if !same_kid {
            return Err(Error::InvalidInput(String::from("invalid keyset id")));
        }
        let target = operation
            .target
            .checked_mul(self.multiplier)
            .unwrap_or(operation.target);
        if operation.minted + output_amount > target {
            return Err(Error::InvalidInput(String::from("exceeding amount")));
        }
        // the first mint carries the mint fee, the minted update below lets only one request through
        let fee = if operation.minted == cashu::Amount::ZERO && operation.fee > cashu::Amount::ZERO
        {
            Some(self.sign_fee(operation.kid, operation.fee).await?)
        } else {
            None
        };
        let signatures = self.wildcatcl.sign(&request.outputs).await?;
        self.repo
            .mint_update_field(
                operation.uid,
                operation.minted,
                operation.minted + output_amount,
            )
            .await?;
        let user_signatures = signatures.len();
        let mut reported_amount = output_amount;
        let mut reported = signatures;
        if let Some((fee_signatures, _)) = &fee {
            reported_amount += operation.fee;
            reported.extend(fee_signatures.iter().cloned());
        }
        let mut signatures = self
            .report_minting(
                operation.kid,
                request.quote,
                reported_amount,
                operation.bill_id,
                reported,
            )
            .await?;
        signatures.truncate(user_signatures);
        if let Some((_, fee_proofs)) = fee {
            if let Err(e) = vault.store_proofs(fee_proofs).await {
                tracing::error!(
                    "failed to store fee proofs for mintop {}: {e}",
                    operation.uid
                );
            }
        }
        Ok(wire_mint::EbillMintResponse { signatures })
    }

    pub async fn request_to_pay_ebill(
        &self,
        bid: BillId,
        amount: bitcoin::Amount,
        deadline: TStamp,
    ) -> Result<()> {
        let (block_id, previous_block_hash) =
            self.wildcatcl.prepare_request_to_pay(bid.clone()).await?;
        let payment_address = self
            .clowdercl
            .request_onchain_ebill_address(bid.clone(), block_id, previous_block_hash)
            .await?;
        let _bill_private_key = self
            .wildcatcl
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
    use crate::ebill::{MockClowderClient, MockRepository, MockVaultService, MockWildcatClient};
    use bcr_common::{cashu, core_tests};
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
            .returning(|_| Err(Error::InvalidInput(String::new())));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            wildcatcl: Box::new(core_cl),
            repo: Box::new(repo),
            multiplier: cashu::Amount::ONE,
        };
        let now = time::OffsetDateTime::now_utc();
        let err = service
            .new_minting_operation(uid, kid.into(), pub_key, amount, amount, bill_id, now)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    /// a fee-less mint operation and the keyset info `new_minting_operation` checks
    fn new_op() -> (ecash::MintKeySetInfo, MintOperation) {
        let (kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let op = MintOperation {
            uid: Uuid::new_v4(),
            kid: kinfo.id.into(),
            pub_key: bcr_common::core::generate_random_keypair()
                .public_key()
                .into(),
            target: cashu::Amount::from(64),
            minted: cashu::Amount::ZERO,
            bill_id: core_tests::random_bill_id(),
            fee: cashu::Amount::ZERO,
        };
        (kinfo, op)
    }

    fn new_op_service(
        kinfo: ecash::MintKeySetInfo,
        calls: usize,
        repo: MockRepository,
        clowder_cl: MockClowderClient,
    ) -> Service {
        let mut core_cl = MockWildcatClient::new();
        core_cl
            .expect_info()
            .times(calls)
            .with(eq(cashu::Id::from(kinfo.id)))
            .returning(move |_| Ok(kinfo.clone().into()));
        Service {
            clowdercl: Box::new(clowder_cl),
            wildcatcl: Box::new(core_cl),
            repo: Box::new(repo),
            multiplier: cashu::Amount::ONE,
        }
    }

    async fn fire_new_op(service: &Service, op: &MintOperation) -> Result<()> {
        let now = time::OffsetDateTime::now_utc();
        service
            .new_minting_operation(
                op.uid,
                op.kid,
                op.pub_key,
                op.target + op.fee,
                op.target,
                op.bill_id.clone(),
                now,
            )
            .await
    }

    fn expect_store_exists(repo: &mut MockRepository, existing: Option<MintOperation>) {
        repo.expect_mint_store()
            .times(1)
            .returning(|op| Err(Error::AlreadyExists(op.uid.to_string())));
        repo.expect_mint_lookup_by_bill()
            .times(1)
            .returning(move |_| Ok(existing.clone()));
    }

    #[tokio::test]
    async fn new_minting_operation_ok() {
        let (kinfo, op) = new_op();
        let mut repo = MockRepository::new();
        let mut clowder_cl = MockClowderClient::new();
        repo.expect_mint_store()
            .times(1)
            .with(eq(op.clone()))
            .returning(|_| Ok(()));
        clowder_cl
            .expect_register_ebill()
            .times(1)
            .with(eq(op.bill_id.clone()), eq(op.target))
            .returning(|_, _| Ok(()));
        let service = new_op_service(kinfo, 1, repo, clowder_cl);
        fire_new_op(&service, &op).await.unwrap();
    }

    #[tokio::test]
    async fn new_minting_operation_refire_same_op_is_idempotent() {
        let (kinfo, op) = new_op();
        let mut repo = MockRepository::new();
        let mut clowder_cl = MockClowderClient::new();
        expect_store_exists(&mut repo, Some(op.clone()));
        clowder_cl
            .expect_register_ebill()
            .times(1)
            .with(eq(op.bill_id.clone()), eq(op.target))
            .returning(|_, _| Ok(()));
        let service = new_op_service(kinfo, 1, repo, clowder_cl);
        fire_new_op(&service, &op).await.unwrap();
    }

    #[tokio::test]
    async fn new_minting_operation_other_op_same_bill_rejected() {
        let (kinfo, op) = new_op();
        let mut repo = MockRepository::new();
        let other = MintOperation {
            uid: Uuid::new_v4(),
            ..op.clone()
        };
        expect_store_exists(&mut repo, Some(other));
        // no register_ebill expectation
        let service = new_op_service(kinfo, 1, repo, MockClowderClient::new());
        let err = fire_new_op(&service, &op).await.unwrap_err();
        assert!(matches!(err, Error::AlreadyExists(_)));
    }

    #[tokio::test]
    async fn new_minting_operation_refire_same_uid_different_params_rejected() {
        let (kinfo, op) = new_op();
        let mut repo = MockRepository::new();
        let existing = MintOperation {
            target: cashu::Amount::from(128),
            ..op.clone()
        };
        expect_store_exists(&mut repo, Some(existing));
        let service = new_op_service(kinfo, 1, repo, MockClowderClient::new());
        let err = fire_new_op(&service, &op).await.unwrap_err();
        assert!(matches!(err, Error::AlreadyExists(_)));
    }

    #[tokio::test]
    async fn new_minting_operation_same_uid_other_bill_rejected() {
        let (kinfo, op) = new_op();
        let mut repo = MockRepository::new();
        // the uid is taken by an operation for another bill, nothing is bound to this bill
        expect_store_exists(&mut repo, None);
        let service = new_op_service(kinfo, 1, repo, MockClowderClient::new());
        let err = fire_new_op(&service, &op).await.unwrap_err();
        assert!(matches!(err, Error::AlreadyExists(_)));
    }

    #[tokio::test]
    async fn new_minting_operation_register_failure_heals_on_refire() {
        let (kinfo, op) = new_op();
        let mut repo = MockRepository::new();
        let mut clowder_cl = MockClowderClient::new();
        repo.expect_mint_store()
            .times(1)
            .with(eq(op.clone()))
            .returning(|_| Ok(()));
        expect_store_exists(&mut repo, Some(op.clone()));
        clowder_cl
            .expect_register_ebill()
            .times(1)
            .with(eq(op.bill_id.clone()), eq(op.target))
            .returning(|_, _| Err(clowder_transport_error()));
        clowder_cl
            .expect_register_ebill()
            .times(1)
            .with(eq(op.bill_id.clone()), eq(op.target))
            .returning(|_, _| Ok(()));
        let service = new_op_service(kinfo, 2, repo, clowder_cl);
        let err = fire_new_op(&service, &op).await.unwrap_err();
        assert!(matches!(err, Error::ClowderNatsClient(_)));
        fire_new_op(&service, &op).await.unwrap();
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
            fee: cashu::Amount::ZERO,
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

        let service = Service {
            clowdercl: Box::new(clowder_cl),
            wildcatcl: Box::new(core_cl),
            repo: Box::new(mintop_repo),
            multiplier: cashu::Amount::ONE,
        };
        let outputs = signatures_test::generate_blinds(kid.into(), &amounts);
        let blinds = outputs
            .iter()
            .map(|(blind, _, _)| blind.clone())
            .map(cashu::BlindedMessage::from)
            .collect();
        let request = wire_mint::EbillMintRequest::new(uid, blinds, &kp);
        let response = service
            .mint(request, &MockVaultService::new())
            .await
            .unwrap();
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
            .returning(move |_| Err(Error::InvalidInput(String::new())));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            wildcatcl: Box::new(core_cl),
            repo: Box::new(mintop_repo),
            multiplier: cashu::Amount::ONE,
        };
        let outputs = signatures_test::generate_blinds(kid.into(), &amounts);
        let blinds = outputs
            .iter()
            .map(|(blind, _, _)| blind.clone())
            .map(cashu::BlindedMessage::from)
            .collect();
        let request = wire_mint::EbillMintRequest::new(uid, blinds, &kp);
        let err = service
            .mint(request, &MockVaultService::new())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    fn clowder_transport_error() -> Error {
        Error::ClowderNatsClient(
            bcr_common::client::clowder::ClowderClientError::Serialization(String::from("timeout")),
        )
    }

    fn mint_retry_service(
        clowder_cl: MockClowderClient,
    ) -> (Service, wire_mint::EbillMintRequest, usize) {
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
            fee: cashu::Amount::ZERO,
        };
        mintop_repo
            .expect_mint_load()
            .times(1)
            .with(eq(uid))
            .returning(move |_| Ok(mintop.clone()));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            wildcatcl: Box::new(core_cl),
            repo: Box::new(mintop_repo),
            multiplier: cashu::Amount::ONE,
        };
        let outputs = signatures_test::generate_blinds(kid.into(), &amounts);
        let blinds = outputs
            .iter()
            .map(|(blind, _, _)| blind.clone())
            .map(cashu::BlindedMessage::from)
            .collect();
        let request = wire_mint::EbillMintRequest::new(uid, blinds, &kp);
        (service, request, amounts.len())
    }

    #[tokio::test]
    async fn mint_clowder_error_keeps_minted_and_returns_error() {
        let mut clowder_cl = MockClowderClient::new();
        clowder_cl
            .expect_minting_ebill()
            .times(MINTING_EBILL_ATTEMPTS as usize)
            .returning(|_, _, _, _, _| Err(clowder_transport_error()));
        let (service, request, _) = mint_retry_service(clowder_cl);
        let err = service
            .mint(request, &MockVaultService::new())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::ClowderNatsClient(_)));
    }

    #[tokio::test]
    async fn mint_clowder_rejection_not_retried() {
        let mut clowder_cl = MockClowderClient::new();
        clowder_cl
            .expect_minting_ebill()
            .times(1)
            .returning(|_, _, _, _, _| {
                Err(Error::ClowderNatsClient(ClowderClientError::Rejected(
                    bcr_common::wire::clowder::ClowderRejection::Expired,
                )))
            });
        let (service, request, _) = mint_retry_service(clowder_cl);
        let err = service
            .mint(request, &MockVaultService::new())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::ClowderNatsClient(ClowderClientError::Rejected(_))
        ));
    }

    #[tokio::test]
    async fn mint_clowder_error_then_ok() {
        let mut clowder_cl = MockClowderClient::new();
        clowder_cl
            .expect_minting_ebill()
            .times(1)
            .returning(|_, _, _, _, _| Err(clowder_transport_error()));
        clowder_cl
            .expect_minting_ebill()
            .times(1)
            .returning(|_, _, _, _, signatures| Ok(signatures));
        let (service, request, signature_count) = mint_retry_service(clowder_cl);
        let response = service
            .mint(request, &MockVaultService::new())
            .await
            .unwrap();
        assert_eq!(response.signatures.len(), signature_count);
    }

    fn sum_signatures(signatures: &[cashu::BlindSignature]) -> cashu::Amount {
        signatures
            .iter()
            .fold(cashu::Amount::ZERO, |acc, s| acc + s.amount)
    }

    fn sum_proofs(proofs: &[cashu::Proof]) -> cashu::Amount {
        proofs
            .iter()
            .fold(cashu::Amount::ZERO, |acc, p| acc + p.amount)
    }

    /// mintop with target 64 and fee 36, the wallet mints `amounts` after `minted`
    struct FeeFixture {
        keyset: ecash::MintKeySet,
        kid: cashu::Id,
        kp: bitcoin::secp256k1::Keypair,
        mintop: MintOperation,
        amounts: Vec<cashu::Amount>,
    }

    impl FeeFixture {
        fn new(minted: cashu::Amount) -> Self {
            let (kinfo, keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
            let kid = cashu::Id::from(kinfo.id);
            let kp = bcr_common::core::generate_random_keypair();
            let mintop = MintOperation {
                uid: Uuid::new_v4(),
                kid,
                pub_key: cashu::PublicKey::from(kp.public_key()),
                target: cashu::Amount::from(64),
                minted,
                bill_id: core_tests::random_bill_id(),
                fee: cashu::Amount::from(36),
            };
            Self {
                keyset,
                kid,
                kp,
                mintop,
                amounts: vec![cashu::Amount::from(16), cashu::Amount::from(8)],
            }
        }

        fn output(&self) -> cashu::Amount {
            self.amounts
                .iter()
                .fold(cashu::Amount::ZERO, |acc, a| acc + *a)
        }

        fn request(&self) -> wire_mint::EbillMintRequest {
            let blinds = signatures_test::generate_blinds(self.kid, &self.amounts)
                .into_iter()
                .map(|(blind, _, _)| cashu::BlindedMessage::from(blind))
                .collect();
            wire_mint::EbillMintRequest::new(self.mintop.uid, blinds, &self.kp)
        }

        /// repo with the mintop and the expected minted update, core signs `sign_calls` times
        fn mocks(&self, sign_calls: usize) -> (MockRepository, MockWildcatClient) {
            let mut repo = MockRepository::new();
            let mintop = self.mintop.clone();
            repo.expect_mint_load()
                .times(1)
                .returning(move |_| Ok(mintop.clone()));
            let minted = self.mintop.minted;
            repo.expect_mint_update_field()
                .times(1)
                .with(eq(self.mintop.uid), eq(minted), eq(minted + self.output()))
                .returning(|_, _, _| Ok(()));
            let mut core_cl = MockWildcatClient::new();
            let keyset = self.keyset.clone();
            core_cl
                .expect_sign()
                .times(sign_calls)
                .returning(move |msgs| {
                    let amounts: Vec<cashu::Amount> = msgs.iter().map(|m| m.amount).collect();
                    Ok(core_tests::generate_ecash_signatures(&keyset, &amounts))
                });
            (repo, core_cl)
        }

        fn expect_keys(&self, core_cl: &mut MockWildcatClient) {
            let keyset = self.keyset.clone();
            core_cl
                .expect_keys()
                .times(1)
                .with(eq(self.kid))
                .returning(move |_| Ok(ecash::KeySet::from(keyset.clone())));
        }

        fn service(
            repo: MockRepository,
            core_cl: MockWildcatClient,
            clowder_cl: MockClowderClient,
        ) -> Service {
            Service {
                clowdercl: Box::new(clowder_cl),
                wildcatcl: Box::new(core_cl),
                repo: Box::new(repo),
                multiplier: cashu::Amount::ONE,
            }
        }
    }

    #[tokio::test]
    async fn new_minting_operation_stores_fee_and_registers_bill_sum() {
        let fx = FeeFixture::new(cashu::Amount::ZERO);
        let amount = fx.mintop.target + fx.mintop.fee;
        let mut repo = MockRepository::new();
        let mut core_cl = MockWildcatClient::new();
        let mut clowder_cl = MockClowderClient::new();
        let (kinfo, _) = bcr_common::core_tests::generate_random_ecash_keyset();
        core_cl
            .expect_info()
            .times(1)
            .returning(move |_| Ok(kinfo.clone().into()));
        repo.expect_mint_store()
            .times(1)
            .with(eq(fx.mintop.clone()))
            .returning(|_| Ok(()));
        clowder_cl
            .expect_register_ebill()
            .times(1)
            .with(eq(fx.mintop.bill_id.clone()), eq(amount))
            .returning(|_, _| Ok(()));
        // nothing is signed nor reported before the wallet mints
        let service = FeeFixture::service(repo, core_cl, clowder_cl);
        let now = time::OffsetDateTime::now_utc();
        service
            .new_minting_operation(
                fx.mintop.uid,
                fx.kid,
                fx.mintop.pub_key,
                amount,
                fx.mintop.target,
                fx.mintop.bill_id.clone(),
                now,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn new_minting_operation_discounted_above_amount_rejected() {
        let fx = FeeFixture::new(cashu::Amount::ZERO);
        let mut core_cl = MockWildcatClient::new();
        let (kinfo, _) = bcr_common::core_tests::generate_random_ecash_keyset();
        core_cl
            .expect_info()
            .times(1)
            .returning(move |_| Ok(kinfo.clone().into()));
        let service = FeeFixture::service(MockRepository::new(), core_cl, MockClowderClient::new());
        let now = time::OffsetDateTime::now_utc();
        let err = service
            .new_minting_operation(
                fx.mintop.uid,
                fx.kid,
                fx.mintop.pub_key,
                fx.mintop.target,
                fx.mintop.target + cashu::Amount::ONE,
                fx.mintop.bill_id.clone(),
                now,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[tokio::test]
    async fn mint_first_mint_reports_fee_with_wallet_signatures() {
        let fx = FeeFixture::new(cashu::Amount::ZERO);
        let (repo, mut core_cl) = fx.mocks(2);
        fx.expect_keys(&mut core_cl);
        let mut clowder_cl = MockClowderClient::new();
        let (kid, uid, bill_id) = (fx.kid, fx.mintop.uid, fx.mintop.bill_id.clone());
        let reported = fx.output() + fx.mintop.fee;
        let wallet_count = fx.amounts.len();
        clowder_cl
            .expect_minting_ebill()
            .times(1)
            .withf(move |k, q, a, b, signatures| {
                *k == kid
                    && *q == uid
                    && *a == reported
                    && *b == bill_id
                    && sum_signatures(signatures) == reported
                    && signatures.len() > wallet_count
            })
            .returning(|_, _, _, _, signatures| Ok(signatures));
        let mut vault = MockVaultService::new();
        let fee = fx.mintop.fee;
        vault
            .expect_store_proofs()
            .times(1)
            .withf(move |proofs| sum_proofs(proofs) == fee)
            .returning(|_| Ok(()));
        let service = FeeFixture::service(repo, core_cl, clowder_cl);

        let response = service.mint(fx.request(), &vault).await.unwrap();
        assert_eq!(response.signatures.len(), fx.amounts.len());
        assert_eq!(sum_signatures(&response.signatures), fx.output());
    }

    #[tokio::test]
    async fn mint_later_mint_carries_no_fee() {
        let fx = FeeFixture::new(cashu::Amount::from(8));
        let (repo, core_cl) = fx.mocks(1);
        let mut clowder_cl = MockClowderClient::new();
        clowder_cl
            .expect_minting_ebill()
            .times(1)
            .with(
                eq(fx.kid),
                eq(fx.mintop.uid),
                eq(fx.output()),
                eq(fx.mintop.bill_id.clone()),
                mockall::predicate::always(),
            )
            .returning(|_, _, _, _, signatures| Ok(signatures));
        // no keys/store_proofs expectations
        let service = FeeFixture::service(repo, core_cl, clowder_cl);

        let response = service
            .mint(fx.request(), &MockVaultService::new())
            .await
            .unwrap();
        assert_eq!(response.signatures.len(), fx.amounts.len());
    }

    #[tokio::test]
    async fn mint_fee_vault_failure_does_not_fail_the_mint() {
        let fx = FeeFixture::new(cashu::Amount::ZERO);
        let (repo, mut core_cl) = fx.mocks(2);
        fx.expect_keys(&mut core_cl);
        let mut clowder_cl = MockClowderClient::new();
        clowder_cl
            .expect_minting_ebill()
            .times(1)
            .returning(|_, _, _, _, signatures| Ok(signatures));
        let mut vault = MockVaultService::new();
        vault
            .expect_store_proofs()
            .times(1)
            .returning(|_| Err(Error::DB(anyhow::anyhow!("down"))));
        let service = FeeFixture::service(repo, core_cl, clowder_cl);

        let response = service.mint(fx.request(), &vault).await.unwrap();
        assert_eq!(response.signatures.len(), fx.amounts.len());
    }

    #[tokio::test]
    async fn mint_empty_outputs_rejected() {
        let mut fx = FeeFixture::new(cashu::Amount::ZERO);
        fx.amounts.clear();
        // no repo/core/clowder/vault expectations: nothing is loaded, signed or reported
        let service = FeeFixture::service(
            MockRepository::new(),
            MockWildcatClient::new(),
            MockClowderClient::new(),
        );

        let err = service
            .mint(fx.request(), &MockVaultService::new())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[tokio::test]
    async fn mint_concurrent_first_mint_loser_reports_nothing() {
        let fx = FeeFixture::new(cashu::Amount::ZERO);
        let mut repo = MockRepository::new();
        let mintop = fx.mintop.clone();
        repo.expect_mint_load()
            .times(1)
            .returning(move |_| Ok(mintop.clone()));
        // the other request already moved minted away from zero
        repo.expect_mint_update_field()
            .times(1)
            .returning(|uid, old, _| {
                Err(Error::InvalidInput(format!(
                    "mintop {uid} and {old} amount not found"
                )))
            });
        let mut core_cl = MockWildcatClient::new();
        fx.expect_keys(&mut core_cl);
        let keyset = fx.keyset.clone();
        core_cl.expect_sign().times(2).returning(move |msgs| {
            let amounts: Vec<cashu::Amount> = msgs.iter().map(|m| m.amount).collect();
            Ok(core_tests::generate_ecash_signatures(&keyset, &amounts))
        });
        // no minting_ebill/store_proofs expectations
        let service = FeeFixture::service(repo, core_cl, MockClowderClient::new());

        let err = service
            .mint(fx.request(), &MockVaultService::new())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }
}
