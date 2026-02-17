// ----- standard library imports
use std::collections::HashSet;
// ----- extra library imports
use bcr_common::{
    cashu,
    cdk_common::mint::MintKeySetInfo,
    core::{self, BillId},
};
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    keys::{factory::Factory, ClowderClient},
    persistence::{KeysRepository, MintOpRepository, SignaturesRepository},
};

// ----- end imports

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MintOperation {
    pub uid: Uuid,
    pub kid: cashu::Id,
    pub pub_key: cashu::PublicKey,
    pub target: cashu::Amount,
    pub minted: cashu::Amount,
    pub bill_id: BillId,
}

pub struct Service {
    pub keys: Box<dyn KeysRepository>,
    pub mintops: Box<dyn MintOpRepository>,
    pub signatures: Box<dyn SignaturesRepository>,
    pub clowder: Box<dyn ClowderClient>,
    pub keygen: Factory,
}

impl Service {
    pub async fn get_keyset_id_for_date(&self, date: chrono::NaiveDate) -> Result<cashu::Id> {
        let datetime = date
            .and_hms_opt(0, 0, 0)
            .expect("get_keyset_id_for_date with 00:00:00 time")
            .and_utc();
        let tstamp = std::cmp::max(datetime.timestamp(), 0) as u64;
        let infos = self.keys.infos_for_expiration_date(tstamp).await?;
        let info = infos
            .iter()
            .find(|info| info.final_expiry.unwrap_or_default() == tstamp);
        if let Some(info) = info {
            return Ok(info.id);
        }
        let new_keyset = self.keygen.generate(datetime);
        let kid = new_keyset.0.id;
        let kset = new_keyset.1.clone();
        self.keys.store(new_keyset).await?;
        self.clowder.new_keyset(cashu::KeySet::from(kset)).await?;
        Ok(kid)
    }

    pub async fn info(&self, kid: cashu::Id) -> Result<MintKeySetInfo> {
        self.keys.info(kid).await?.ok_or(Error::KeysetNotFound(kid))
    }

    pub async fn keys(&self, kid: cashu::Id) -> Result<cashu::MintKeySet> {
        self.keys
            .keyset(kid)
            .await?
            .ok_or(Error::KeysetNotFound(kid))
    }

    pub async fn verify_proof(&self, proof: cashu::Proof) -> Result<()> {
        let keyset = self.keys(proof.keyset_id).await?;
        core::signature::verify_ecash_proof(&keyset, &proof)?;
        Ok(())
    }

    pub async fn verify_fingerprint(&self, fp: core::signature::ProofFingerprint) -> Result<()> {
        let keyset = self.keys(fp.keyset_id).await?;
        core::signature::verify_ecash_fingerprint(&keyset, &fp)?;
        Ok(())
    }

    pub async fn list_info(&self) -> Result<Vec<MintKeySetInfo>> {
        self.keys.list_info().await
    }

    pub async fn list_keyset(&self) -> Result<Vec<cashu::MintKeySet>> {
        self.keys.list_keyset().await
    }

    pub async fn deactivate(&self, kid: cashu::Id) -> Result<cashu::Id> {
        let mut info = self
            .keys
            .info(kid)
            .await?
            .ok_or(Error::KeysetNotFound(kid))?;
        info.active = false;
        self.keys.update_info(info.clone()).await?;
        self.clowder.keyset_deactivated(kid).await?;
        Ok(info.id)
    }

    pub async fn search_signature(
        &self,
        blind: &cashu::BlindedMessage,
    ) -> Result<Option<cashu::BlindSignature>> {
        self.signatures.load(blind).await
    }

    pub async fn sign_blind(&self, blind: &cashu::BlindedMessage) -> Result<cashu::BlindSignature> {
        let keyset = self.keys(blind.keyset_id).await?;
        let signature = core::signature::sign_ecash(&keyset, blind)?;
        self.signatures
            .store(blind.blinded_secret, signature.clone())
            .await?;
        Ok(signature)
    }

    pub async fn new_minting_operation(
        &self,
        uid: Uuid,
        kid: cashu::Id,
        pub_key: cashu::PublicKey,
        amount: cashu::Amount,
        bill_id: BillId,
    ) -> Result<()> {
        if self.keys.info(kid).await?.is_none() {
            return Err(Error::KeysetNotFound(kid));
        }
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

    pub async fn mint(&self, request: &cashu::MintRequest<Uuid>) -> Result<cashu::MintResponse> {
        // basic checks
        if request.signature.is_none() {
            return Err(Error::InvalidMintRequest(String::from("signature missing")));
        }
        bcr_wdc_utils::signatures::basic_blinds_checks(&request.outputs)
            .map_err(|e| Error::InvalidMintRequest(e.to_string()))?;
        //  check if the ids of the outputs are all the same
        let unique_ids: Vec<_> = request
            .outputs
            .iter()
            .map(|p| p.keyset_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if unique_ids.len() != 1 {
            return Err(Error::InvalidMintRequest(String::from(
                "multiple keyset IDs",
            )));
        }
        let output_amount = request
            .outputs
            .iter()
            .fold(cashu::Amount::ZERO, |acc, blind| acc + blind.amount);
        let kid = unique_ids.first().expect("unique_ids len should be 1");
        let operation = self.mintops.load(request.quote).await?;
        let signature_verification = request.verify_signature(operation.pub_key);
        if signature_verification.is_err() {
            return Err(Error::InvalidMintRequest(String::from(
                "signature verification failed",
            )));
        }
        if operation.minted + output_amount > operation.target {
            return Err(Error::InvalidMintRequest(String::from(
                "outputs amount exceeds allowance",
            )));
        }
        let keyset = self.keys(*kid).await?;
        let mut signatures = Vec::with_capacity(request.outputs.len());
        for blind in &request.outputs {
            let signature = core::signature::sign_ecash(&keyset, blind)?;
            self.signatures
                .store(blind.blinded_secret, signature.clone())
                .await?;
            signatures.push(signature);
        }
        self.mintops
            .update(
                operation.uid,
                operation.minted,
                operation.minted + output_amount,
            )
            .await?;
        let signatures = self
            .clowder
            .mint_ebill(
                keyset.id,
                request.quote,
                output_amount,
                operation.bill_id.clone(),
                signatures,
            )
            .await?;
        Ok(cashu::MintResponse { signatures })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        btc32::DerivationPath,
        keys::MockClowderClient,
        persistence::{MockKeysRepository, MockMintOpRepository, MockSignaturesRepository},
    };
    use bcr_common::core_tests::random_bill_id;
    use bcr_wdc_utils::signatures::test_utils::generate_blinds;
    use cashu::Amount;
    use mockall::predicate::eq;
    use secp256k1::Keypair;
    use std::str::FromStr;

    fn seed() -> [u8; 64] {
        bip39::Mnemonic::from_str(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap().to_seed("")
    }

    #[tokio::test]
    async fn infos_for_expiration_date_existing() {
        let factory = Factory::new(&seed(), DerivationPath::default());
        let mut keys_repo = MockKeysRepository::new();
        let signatures_repo = MockSignaturesRepository::new();
        let mintop_repo = MockMintOpRepository::new();
        let clowder_cl = MockClowderClient::new();
        let maturity = chrono::NaiveDate::from_ymd_opt(2030, 12, 31).unwrap();
        let maturity_timestamp =
            maturity.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp() as u64;
        let (mut kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        kinfo.final_expiry = Some(maturity_timestamp);
        let expected_id = kinfo.id;
        let cloned_info = kinfo.clone();
        keys_repo
            .expect_infos_for_expiration_date()
            .times(1)
            .with(eq(maturity_timestamp))
            .returning(move |_| Ok(vec![cloned_info.clone()]));

        let service = Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            keygen: factory,
            clowder: Box::new(clowder_cl),
            mintops: Box::new(mintop_repo),
        };
        let kid = service.get_keyset_id_for_date(maturity).await.unwrap();
        assert_eq!(kid, expected_id);
    }

    #[tokio::test]
    async fn infos_for_expiration_new_keyset() {
        let expected_factory = Factory::new(&seed(), DerivationPath::default());
        let factory = Factory::new(&seed(), DerivationPath::default());
        let mut keys_repo = MockKeysRepository::new();
        let signatures_repo = MockSignaturesRepository::new();
        let mintop_repo = MockMintOpRepository::new();
        let mut clowder_cl = MockClowderClient::new();
        let maturity = chrono::NaiveDate::from_ymd_opt(2027, 6, 30).unwrap();
        let datetime = maturity.and_hms_opt(0, 0, 0).unwrap().and_utc();
        let maturity_timestamp = datetime.timestamp() as u64;
        let expected_entry = expected_factory.generate(datetime);
        let expected_id = expected_entry.0.id;
        keys_repo
            .expect_infos_for_expiration_date()
            .times(1)
            .with(eq(maturity_timestamp))
            .returning(|_| Ok(Vec::new()));
        keys_repo
            .expect_store()
            .times(1)
            .withf(move |entry| {
                entry.0.id == expected_id && entry.0.final_expiry == Some(maturity_timestamp)
            })
            .returning(|_| Ok(()));
        clowder_cl
            .expect_new_keyset()
            .times(1)
            .returning(|_| Ok(()));
        let service = Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            keygen: factory,
            clowder: Box::new(clowder_cl),
            mintops: Box::new(mintop_repo),
        };
        let kid = service.get_keyset_id_for_date(maturity).await.unwrap();
        assert_eq!(kid, expected_id);
    }

    #[tokio::test]
    async fn deactivate_ok() {
        let factory = Factory::new(&seed(), DerivationPath::default());
        let mut keys_repo = MockKeysRepository::new();
        let signatures_repo = MockSignaturesRepository::new();
        let mintop_repo = MockMintOpRepository::new();
        let mut clowder_cl = MockClowderClient::new();
        let (kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let mut updated_info = kinfo.clone();
        updated_info.active = false;
        keys_repo
            .expect_info()
            .times(1)
            .with(eq(kid))
            .returning(move |_| Ok(Some(kinfo.clone())));
        keys_repo
            .expect_update_info()
            .times(1)
            .with(eq(updated_info.clone()))
            .returning(|_| Ok(()));
        clowder_cl
            .expect_keyset_deactivated()
            .times(1)
            .with(eq(kid))
            .returning(|_| Ok(()));
        let service = Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            keygen: factory,
            clowder: Box::new(clowder_cl),
            mintops: Box::new(mintop_repo),
        };
        let deactivated = service.deactivate(kid).await.unwrap();
        assert_eq!(deactivated, kid);
    }

    #[tokio::test]
    async fn deactivate_no_keysetID() {
        let factory = Factory::new(&seed(), DerivationPath::default());
        let mut keys_repo = MockKeysRepository::new();
        let signatures_repo = MockSignaturesRepository::new();
        let mintop_repo = MockMintOpRepository::new();
        let clowder_cl = MockClowderClient::new();
        let kid = bcr_common::core_tests::generate_random_ecash_keyset().0.id;
        keys_repo
            .expect_info()
            .times(1)
            .with(eq(kid))
            .returning(|_| Ok(None));
        let service = Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            keygen: factory,
            clowder: Box::new(clowder_cl),
            mintops: Box::new(mintop_repo),
        };
        let err = service.deactivate(kid).await.unwrap_err();
        assert!(matches!(err, Error::KeysetNotFound(id) if id == kid));
    }

    #[tokio::test]
    async fn new_minting_operation_missing_keyset() {
        let factory = Factory::new(&seed(), DerivationPath::default());
        let mut keys_repo = MockKeysRepository::new();
        let signatures_repo = MockSignaturesRepository::new();
        let mintop_repo = MockMintOpRepository::new();
        let clowder_cl = MockClowderClient::new();
        let kid = bcr_common::core_tests::generate_random_ecash_keyset().0.id;
        let uid = Uuid::new_v4();
        let pub_key = bcr_common::core_tests::generate_random_keypair()
            .public_key()
            .into();
        let amount = Amount::from(32);
        let bill_id = random_bill_id();
        keys_repo
            .expect_info()
            .times(1)
            .with(eq(kid))
            .returning(|_| Ok(None));
        let service = Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            keygen: factory,
            clowder: Box::new(clowder_cl),
            mintops: Box::new(mintop_repo),
        };
        let err = service
            .new_minting_operation(uid, kid, pub_key, amount, bill_id)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::KeysetNotFound(id) if id == kid));
    }

    #[tokio::test]
    async fn new_minting_operation_ok() {
        let factory = Factory::new(&seed(), DerivationPath::default());
        let mut keys_repo = MockKeysRepository::new();
        let signatures_repo = MockSignaturesRepository::new();
        let mut mintop_repo = MockMintOpRepository::new();
        let clowder_cl = MockClowderClient::new();
        let (kinfo, _keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let uid = Uuid::new_v4();
        let pub_key = bcr_common::core_tests::generate_random_keypair()
            .public_key()
            .into();
        let amount = Amount::from(64);
        let bill_id = random_bill_id();
        keys_repo
            .expect_info()
            .times(1)
            .with(eq(kid))
            .returning(move |_| Ok(Some(kinfo.clone())));
        let mintop = MintOperation {
            uid,
            kid,
            pub_key,
            target: amount,
            minted: Amount::ZERO,
            bill_id: bill_id.clone(),
        };
        mintop_repo
            .expect_store()
            .times(1)
            .with(eq(mintop))
            .returning(|_| Ok(()));
        let service = Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            keygen: factory,
            clowder: Box::new(clowder_cl),
            mintops: Box::new(mintop_repo),
        };
        service
            .new_minting_operation(uid, kid, pub_key, amount, bill_id)
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn mint_ok() {
        let factory = Factory::new(&seed(), DerivationPath::default());
        let mut keys_repo = MockKeysRepository::new();
        let mut signatures_repo = MockSignaturesRepository::new();
        let mut mintop_repo = MockMintOpRepository::new();
        let mut clowder_cl = MockClowderClient::new();
        let (kinfo, keyset) = bcr_common::core_tests::generate_random_ecash_keyset();
        let kid = kinfo.id;
        let uid = Uuid::new_v4();
        let kp = bcr_common::core_tests::generate_random_keypair();
        let pub_key = cashu::PublicKey::from(kp.public_key());
        let amounts = [Amount::from(128), Amount::from(64)];
        let total = amounts
            .iter()
            .fold(Amount::ZERO, |acc, amount| acc + *amount);
        let bill_id = random_bill_id();
        mintop_repo
            .expect_update()
            .times(1)
            .with(eq(uid), eq(Amount::ZERO), eq(total))
            .returning(|_, _, _| Ok(()));
        keys_repo
            .expect_keyset()
            .times(1)
            .with(eq(kid))
            .returning(move |_| Ok(Some(keyset.clone())));
        signatures_repo
            .expect_store()
            .times(amounts.len())
            .returning(|_, _| Ok(()));
        let mintop = MintOperation {
            uid,
            kid,
            pub_key,
            target: total,
            minted: Amount::ZERO,
            bill_id: bill_id.clone(),
        };
        mintop_repo
            .expect_load()
            .times(1)
            .with(eq(uid))
            .returning(move |_| Ok(mintop.clone()));
        clowder_cl
            .expect_mint_ebill()
            .times(1)
            .returning(|_, _, _, _, signatures| Ok(signatures));

        let service = Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            keygen: factory,
            clowder: Box::new(clowder_cl),
            mintops: Box::new(mintop_repo),
        };
        let outputs = generate_blinds(kid, &amounts);
        let blinds = outputs.iter().map(|(blind, _, _)| blind.clone()).collect();
        let mut request = cashu::MintRequest {
            quote: uid,
            outputs: blinds,
            signature: None,
        };
        request.sign(kp.secret_key().into()).unwrap();
        let response = service.mint(&request).await.unwrap();
        assert_eq!(response.signatures.len(), amounts.len());
    }

    #[tokio::test]
    async fn mint_missing_mintop() {
        let factory = Factory::new(&seed(), DerivationPath::default());
        let keys_repo = MockKeysRepository::new();
        let signatures_repo = MockSignaturesRepository::new();
        let mut mintop_repo = MockMintOpRepository::new();
        let clowder_cl = MockClowderClient::new();
        let kid = bcr_common::core_tests::generate_random_ecash_keyset().0.id;
        let uid = Uuid::new_v4();
        let kp = bcr_common::core_tests::generate_random_keypair();
        let amounts = [Amount::from(128)];
        mintop_repo
            .expect_load()
            .times(1)
            .with(eq(uid))
            .returning(move |_| Err(Error::MintOpNotFound(uid)));
        let service = Service {
            keys: Box::new(keys_repo),
            signatures: Box::new(signatures_repo),
            keygen: factory,
            clowder: Box::new(clowder_cl),
            mintops: Box::new(mintop_repo),
        };
        let outputs = generate_blinds(kid, &amounts);
        let blinds = outputs.iter().map(|(blind, _, _)| blind.clone()).collect();
        let mut request = cashu::MintRequest {
            quote: uid,
            outputs: blinds,
            signature: None,
        };
        request.sign(kp.secret_key().into()).unwrap();
        let err = service.mint(&request).await.unwrap_err();
        assert!(matches!(err, Error::MintOpNotFound(id) if id == uid));
    }
}
