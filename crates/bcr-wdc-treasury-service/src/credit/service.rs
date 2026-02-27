// ----- standard library imports
use std::collections::HashSet;
// ----- extra library imports
use bcr_common::{cashu, core::BillId};
use futures::future::JoinAll;
use uuid::Uuid;
// ----- local imports
use crate::{
    credit::{ClowderClient, CoreClient, MintOpRepository, MintOperation},
    error::{Error, Result},
};

// ----- end imports

pub struct Service {
    pub mintops: Box<dyn MintOpRepository>,
    pub corecl: Box<dyn CoreClient>,
    pub clowdercl: Box<dyn ClowderClient>,
}

impl Service {
    pub async fn new_minting_operation(
        &self,
        uid: Uuid,
        kid: cashu::Id,
        pub_key: cashu::PublicKey,
        amount: cashu::Amount,
        bill_id: BillId,
    ) -> Result<()> {
        let _kinfo = self.corecl.info(kid).await?;
        let new = MintOperation {
            uid,
            kid,
            pub_key,
            target: amount,
            minted: cashu::Amount::ZERO,
            bill_id,
        };
        self.mintops.store(new).await?;
        Ok(())
    }

    pub async fn mintop_status(&self, uid: Uuid) -> Result<MintOperation> {
        let operation = self.mintops.load(uid).await?;
        Ok(operation)
    }

    pub async fn list_mintops_for_kid(&self, kid: cashu::Id) -> Result<Vec<MintOperation>> {
        self.mintops.list(kid).await
    }

    pub async fn mint(&self, request: cashu::MintRequest<Uuid>) -> Result<cashu::MintResponse> {
        // basic checks
        if request.signature.is_none() {
            return Err(Error::InvalidInput(String::from("signature missing")));
        }
        bcr_wdc_utils::signatures::basic_blinds_checks(&request.outputs)
            .map_err(|e| Error::InvalidInput(e.to_string()))?;
        //  check if the ids of the outputs are all the same
        let unique_ids: Vec<_> = request
            .outputs
            .iter()
            .map(|p| p.keyset_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if unique_ids.len() != 1 {
            return Err(Error::InvalidInput(String::from("multiple keyset IDs")));
        }
        let kid = unique_ids.first().expect("unique_ids len should be 1");
        let output_amount = request
            .outputs
            .iter()
            .fold(cashu::Amount::ZERO, |acc, blind| acc + blind.amount);
        let operation = self.mintops.load(request.quote).await?;
        let signature_verification = request.verify_signature(operation.pub_key);
        if signature_verification.is_err() {
            return Err(Error::InvalidInput(String::from("invalid signature")));
        }
        if operation.kid != *kid {
            return Err(Error::InvalidInput(String::from("invalid keyset id")));
        }
        if operation.minted + output_amount > operation.target {
            return Err(Error::InvalidInput(String::from("exceeding amount")));
        }
        let joined: JoinAll<_> = request
            .outputs
            .into_iter()
            .map(|blind| self.corecl.sign(blind))
            .collect();
        let signatures: Vec<cashu::BlindSignature> =
            joined.await.into_iter().collect::<Result<_>>()?;
        self.mintops
            .update(
                operation.uid,
                operation.minted,
                operation.minted + output_amount,
            )
            .await?;
        let response = self
            .clowdercl
            .minting_ebill(
                *kid,
                request.quote,
                output_amount,
                operation.bill_id.clone(),
                signatures,
            )
            .await;
        match response {
            Ok(signatures) => Ok(cashu::MintResponse { signatures }),
            Err(e) => {
                self.mintops
                    .update(
                        operation.uid,
                        operation.minted + output_amount,
                        operation.minted,
                    )
                    .await?;
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credit::{MockClowderClient, MockCoreClient, MockMintOpRepository};
    use bcr_common::{cashu, core_tests};
    use bcr_wdc_utils::signatures::test_utils as signatures_test;
    use mockall::predicate::eq;

    #[tokio::test]
    async fn new_minting_operation_missing_keyset() {
        let mintop_repo = MockMintOpRepository::new();
        let clowder_cl = MockClowderClient::new();
        let mut core_cl = MockCoreClient::new();
        let kid = bcr_common::core_tests::generate_random_ecash_keyset().0.id;
        let uid = Uuid::new_v4();
        let pub_key = bcr_common::core_tests::generate_random_keypair()
            .public_key()
            .into();
        let amount = cashu::Amount::from(32);
        let bill_id = core_tests::random_bill_id();
        core_cl
            .expect_info()
            .times(1)
            .with(eq(kid))
            .returning(|_| Err(Error::InvalidInput(String::new())));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            corecl: Box::new(core_cl),
            mintops: Box::new(mintop_repo),
        };
        let err = service
            .new_minting_operation(uid, kid, pub_key, amount, bill_id)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[tokio::test]
    async fn new_minting_operation_ok() {
        let mut mintop_repo = MockMintOpRepository::new();
        let clowder_cl = MockClowderClient::new();
        let mut core_cl = MockCoreClient::new();
        let (kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let uid = Uuid::new_v4();
        let pub_key = bcr_common::core_tests::generate_random_keypair()
            .public_key()
            .into();
        let amount = cashu::Amount::from(64);
        let bill_id = core_tests::random_bill_id();
        core_cl
            .expect_info()
            .times(1)
            .with(eq(kid))
            .returning(move |_| Ok(kinfo.clone().into()));
        let mintop = MintOperation {
            uid,
            kid,
            pub_key,
            target: amount,
            minted: cashu::Amount::ZERO,
            bill_id: bill_id.clone(),
        };
        mintop_repo
            .expect_store()
            .times(1)
            .with(eq(mintop))
            .returning(|_| Ok(()));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            corecl: Box::new(core_cl),
            mintops: Box::new(mintop_repo),
        };
        service
            .new_minting_operation(uid, kid, pub_key, amount, bill_id)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn mint_ok() {
        let mut mintop_repo = MockMintOpRepository::new();
        let mut clowder_cl = MockClowderClient::new();
        let mut core_cl = MockCoreClient::new();
        let (kinfo, keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let uid = Uuid::new_v4();
        let kp = bcr_common::core_tests::generate_random_keypair();
        let pub_key = cashu::PublicKey::from(kp.public_key());
        let amounts = [cashu::Amount::from(128), cashu::Amount::from(64)];
        let total = amounts
            .iter()
            .fold(cashu::Amount::ZERO, |acc, amount| acc + *amount);
        let bill_id = core_tests::random_bill_id();
        mintop_repo
            .expect_update()
            .times(1)
            .with(eq(uid), eq(cashu::Amount::ZERO), eq(total))
            .returning(|_, _, _| Ok(()));
        core_cl.expect_sign().times(2).returning(move |msg| {
            Ok(core_tests::generate_ecash_signatures(&keyset, &[msg.amount])[0].clone())
        });
        let mintop = MintOperation {
            uid,
            kid,
            pub_key,
            target: total,
            minted: cashu::Amount::ZERO,
            bill_id: bill_id.clone(),
        };
        mintop_repo
            .expect_load()
            .times(1)
            .with(eq(uid))
            .returning(move |_| Ok(mintop.clone()));
        clowder_cl
            .expect_minting_ebill()
            .times(1)
            .returning(|_, _, _, _, signatures| Ok(signatures));

        let service = Service {
            clowdercl: Box::new(clowder_cl),
            corecl: Box::new(core_cl),
            mintops: Box::new(mintop_repo),
        };
        let outputs = signatures_test::generate_blinds(kid, &amounts);
        let blinds = outputs.iter().map(|(blind, _, _)| blind.clone()).collect();
        let mut request = cashu::MintRequest {
            quote: uid,
            outputs: blinds,
            signature: None,
        };
        request.sign(kp.secret_key().into()).unwrap();
        let response = service.mint(request).await.unwrap();
        assert_eq!(response.signatures.len(), amounts.len());
    }

    #[tokio::test]
    async fn mint_missing_mintop() {
        let mut mintop_repo = MockMintOpRepository::new();
        let clowder_cl = MockClowderClient::new();
        let core_cl = MockCoreClient::new();
        let kid = bcr_common::core_tests::generate_random_ecash_keyset().0.id;
        let uid = Uuid::new_v4();
        let kp = bcr_common::core_tests::generate_random_keypair();
        let amounts = [cashu::Amount::from(128)];
        mintop_repo
            .expect_load()
            .times(1)
            .with(eq(uid))
            .returning(move |_| Err(Error::InvalidInput(String::new())));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            corecl: Box::new(core_cl),
            mintops: Box::new(mintop_repo),
        };
        let outputs = signatures_test::generate_blinds(kid, &amounts);
        let blinds = outputs.iter().map(|(blind, _, _)| blind.clone()).collect();
        let mut request = cashu::MintRequest {
            quote: uid,
            outputs: blinds,
            signature: None,
        };
        request.sign(kp.secret_key().into()).unwrap();
        let err = service.mint(request).await.unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }
}
