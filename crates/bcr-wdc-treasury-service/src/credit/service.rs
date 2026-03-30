// ----- standard library imports
// ----- extra library imports
use bcr_common::{
    cashu::{self, ProofsMethods},
    core::BillId,
};
use bcr_wdc_utils::routine::TStamp;
use uuid::Uuid;
// ----- local imports
use crate::{
    credit::{ClowderClient, MeltOperation, MintOperation, Repository, WildcatClient},
    error::{Error, Result},
};

// ----- end imports

pub struct Service {
    pub repo: Box<dyn Repository>,
    pub wildcatcl: Box<dyn WildcatClient>,
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
        let existing = self.repo.melt_load(kid).await;
        match existing {
            Ok(_) => {
                return Err(Error::InvalidInput(format!(
                    "{kid} already melting, cannot create new mint operation"
                )));
            }
            Err(Error::UnknownKeyset(_)) => {
                // this is the expected case, we want to create a mintop for a new kid
            }
            Err(e) => {
                return Err(e);
            }
        }
        let _kinfo = self.wildcatcl.info(kid).await?;
        let new = MintOperation {
            uid,
            kid,
            pub_key,
            target: amount,
            minted: cashu::Amount::ZERO,
            bill_id,
        };
        self.repo.mint_store(new).await?;
        Ok(())
    }

    pub async fn mintop_status(&self, uid: Uuid) -> Result<MintOperation> {
        let operation = self.repo.mint_load(uid).await?;
        Ok(operation)
    }

    pub async fn list_mintops_for_kid(&self, kid: cashu::Id) -> Result<Vec<MintOperation>> {
        self.repo.mint_list(kid).await
    }

    pub async fn mint(&self, request: cashu::MintRequest<Uuid>) -> Result<cashu::MintResponse> {
        // basic checks
        if request.signature.is_none() {
            return Err(Error::InvalidInput(String::from("signature missing")));
        }
        bcr_wdc_utils::signatures::basic_blinds_checks(&request.outputs)
            .map_err(|e| Error::InvalidInput(e.to_string()))?;
        let output_amount = request
            .outputs
            .iter()
            .fold(cashu::Amount::ZERO, |acc, blind| acc + blind.amount);
        let operation = self.repo.mint_load(request.quote).await?;
        let signature_verification = request.verify_signature(operation.pub_key);
        if signature_verification.is_err() {
            return Err(Error::InvalidInput(String::from("invalid signature")));
        }
        let same_kid = request
            .outputs
            .iter()
            .all(|blind| blind.keyset_id == operation.kid);
        if !same_kid {
            return Err(Error::InvalidInput(String::from("invalid keyset id")));
        }
        if operation.minted + output_amount > operation.target {
            return Err(Error::InvalidInput(String::from("exceeding amount")));
        }
        let signatures = self.wildcatcl.sign(&request.outputs).await?;
        self.repo
            .mint_update_field(
                operation.uid,
                operation.minted,
                operation.minted + output_amount,
            )
            .await?;
        let response = self
            .clowdercl
            .minting_ebill(
                operation.kid,
                request.quote,
                output_amount,
                operation.bill_id.clone(),
                signatures,
            )
            .await;
        match response {
            Ok(signatures) => Ok(cashu::MintResponse { signatures }),
            Err(e) => {
                self.repo
                    .mint_update_field(
                        operation.uid,
                        operation.minted + output_amount,
                        operation.minted,
                    )
                    .await?;
                Err(e)
            }
        }
    }

    pub async fn new_meltop(&self, kid: cashu::Id) -> Result<()> {
        let _kinfo = self.wildcatcl.info(kid).await?;
        let new = MeltOperation {
            kid,
            melted: cashu::Amount::ZERO,
        };
        self.repo.melt_store(new).await?;
        Ok(())
    }

    pub async fn meltop_status(&self, kid: cashu::Id) -> Result<MeltOperation> {
        let operation = self.repo.melt_load(kid).await?;
        Ok(operation)
    }

    /// return total melted for cashu::Id
    pub async fn melt(&self, proofs: Vec<cashu::Proof>) -> Result<cashu::Amount> {
        if proofs.is_empty() {
            return Ok(cashu::Amount::ZERO);
        }
        let unique_kid = proofs
            .iter()
            .map(|proof| proof.keyset_id)
            .collect::<std::collections::HashSet<_>>();
        if unique_kid.len() != 1 {
            return Err(Error::InvalidInput(String::from("no unique keyset id")));
        }
        let kid = unique_kid.into_iter().next().unwrap();
        let meltop = self.repo.melt_load(kid).await?;
        let proofs_amount = proofs.total_amount()?;
        let new_melt = meltop.melted + proofs_amount;
        self.repo
            .melt_update_field(kid, meltop.melted, new_melt)
            .await?;
        let result = self.wildcatcl.burn(proofs).await;
        if let Err(e) = result {
            tracing::warn!("burn failed, reverting melt update for {kid}");
            let revert_result = self
                .repo
                .melt_update_field(kid, new_melt, meltop.melted)
                .await;
            if revert_result.is_err() {
                tracing::error!("failed to revert melt update for {kid}, inconsistent state");
            }
            return Err(e);
        }
        Ok(new_melt)
    }

    pub async fn request_to_pay_ebill(
        &self,
        _bid: BillId,
        _amount: bitcoin::Amount,
        _deadline: TStamp,
    ) -> Result<(Uuid, String)> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credit::{MockClowderClient, MockRepository, MockWildcatClient};
    use bcr_common::{cashu, core_tests};
    use bcr_wdc_utils::signatures::test_utils as signatures_test;
    use mockall::predicate::eq;

    #[tokio::test]
    async fn new_minting_operation_missing_keyset() {
        let mut repo = MockRepository::new();
        let clowder_cl = MockClowderClient::new();
        let mut core_cl = MockWildcatClient::new();
        let kid = bcr_common::core_tests::generate_random_ecash_keyset().0.id;
        let uid = Uuid::new_v4();
        let pub_key = bcr_common::core_tests::generate_random_keypair()
            .public_key()
            .into();
        let amount = cashu::Amount::from(32);
        let bill_id = core_tests::random_bill_id();
        repo.expect_melt_load()
            .times(1)
            .with(eq(kid))
            .returning(move |_| Err(Error::UnknownKeyset(kid)));
        core_cl
            .expect_info()
            .times(1)
            .with(eq(kid))
            .returning(|_| Err(Error::InvalidInput(String::new())));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            wildcatcl: Box::new(core_cl),
            repo: Box::new(repo),
        };
        let err = service
            .new_minting_operation(uid, kid, pub_key, amount, bill_id)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[tokio::test]
    async fn new_minting_operation_ok() {
        let mut repo = MockRepository::new();
        let clowder_cl = MockClowderClient::new();
        let mut core_cl = MockWildcatClient::new();
        let (kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let uid = Uuid::new_v4();
        let pub_key = bcr_common::core_tests::generate_random_keypair()
            .public_key()
            .into();
        let amount = cashu::Amount::from(64);
        let bill_id = core_tests::random_bill_id();
        repo.expect_melt_load()
            .times(1)
            .with(eq(kid))
            .returning(move |_| Err(Error::UnknownKeyset(kid)));
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
        repo.expect_mint_store()
            .times(1)
            .with(eq(mintop))
            .returning(|_| Ok(()));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            wildcatcl: Box::new(core_cl),
            repo: Box::new(repo),
        };
        service
            .new_minting_operation(uid, kid, pub_key, amount, bill_id)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn mint_ok() {
        let mut mintop_repo = MockRepository::new();
        let mut clowder_cl = MockClowderClient::new();
        let mut core_cl = MockWildcatClient::new();
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
            kid,
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

        let service = Service {
            clowdercl: Box::new(clowder_cl),
            wildcatcl: Box::new(core_cl),
            repo: Box::new(mintop_repo),
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
        let mut mintop_repo = MockRepository::new();
        let clowder_cl = MockClowderClient::new();
        let core_cl = MockWildcatClient::new();
        let kid = bcr_common::core_tests::generate_random_ecash_keyset().0.id;
        let uid = Uuid::new_v4();
        let kp = bcr_common::core_tests::generate_random_keypair();
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

    #[tokio::test]
    async fn melt_multiple_kids() {
        let repo = MockRepository::new();
        let clowder_cl = MockClowderClient::new();
        let core_cl = MockWildcatClient::new();
        let (_, keyset1) = bcr_common::core_tests::generate_random_ecash_keyset();
        let (_, keyset2) = bcr_common::core_tests::generate_random_ecash_keyset();
        let amounts = [cashu::Amount::from(32)];
        let mut proofs = core_tests::generate_random_ecash_proofs(&keyset1, &amounts);
        let proof2 = core_tests::generate_random_ecash_proofs(&keyset2, &amounts);
        proofs.extend(proof2);
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            wildcatcl: Box::new(core_cl),
            repo: Box::new(repo),
        };
        service.melt(proofs).await.unwrap_err();
    }

    #[tokio::test]
    async fn melt_ok() {
        let mut repo = MockRepository::new();
        let clowder_cl = MockClowderClient::new();
        let mut core_cl = MockWildcatClient::new();
        let (_, keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = keyset.id;
        let amounts = [cashu::Amount::from(32), cashu::Amount::from(64)];
        let total = amounts
            .iter()
            .fold(cashu::Amount::ZERO, |acc, amount| acc + *amount);
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &amounts);
        repo.expect_melt_load()
            .times(1)
            .with(eq(kid))
            .returning(move |_| {
                Ok(MeltOperation {
                    kid,
                    melted: cashu::Amount::ZERO,
                })
            });
        repo.expect_melt_update_field()
            .times(1)
            .with(eq(kid), eq(cashu::Amount::ZERO), eq(total))
            .returning(|_, _, _| Ok(()));
        core_cl
            .expect_burn()
            .times(1)
            .with(eq(proofs.clone()))
            .returning(|_| Ok(()));
        let service = Service {
            clowdercl: Box::new(clowder_cl),
            wildcatcl: Box::new(core_cl),
            repo: Box::new(repo),
        };
        let melted = service.melt(proofs).await.unwrap();
        assert_eq!(melted, total);
    }

    #[tokio::test]
    async fn melt_empty_proofs() {
        let repo = MockRepository::new();
        let core_cl = MockWildcatClient::new();
        let clowder_cl = MockClowderClient::new();
        let service = Service {
            repo: Box::new(repo),
            wildcatcl: Box::new(core_cl),
            clowdercl: Box::new(clowder_cl),
        };
        let result = service.melt(Vec::new()).await.unwrap();
        assert_eq!(result, cashu::Amount::ZERO);
    }
}
