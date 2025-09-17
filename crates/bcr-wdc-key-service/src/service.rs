// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_utils::keys as keys_utils;
use cdk_common::mint::MintKeySetInfo;
use itertools::Itertools;
use uuid::Uuid;
// ----- local imports
use crate::error::{Error, Result};
use crate::factory::Factory;
use crate::TStamp;

// ----- end imports

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[cfg_attr(test, derive(PartialEq))]
pub struct MintOperation {
    pub uid: Uuid,
    pub kid: cashu::Id,
    pub pub_key: cashu::PublicKey,
    pub target: cashu::Amount,
    pub minted: cashu::Amount,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeysRepository: Send + Sync {
    async fn store(&self, keys: keys_utils::KeysetEntry) -> Result<()>;
    async fn info(&self, id: cashu::Id) -> Result<Option<MintKeySetInfo>>;
    async fn keyset(&self, id: cashu::Id) -> Result<Option<cashu::MintKeySet>>;
    async fn list_info(&self) -> Result<Vec<MintKeySetInfo>>;
    async fn list_keyset(&self) -> Result<Vec<cashu::MintKeySet>>;
    async fn update_info(&self, info: MintKeySetInfo) -> Result<()>;
    async fn infos_for_expiration_date(&self, expire: TStamp) -> Result<Vec<MintKeySetInfo>>;
    async fn store_mintop(&self, mint_operation: MintOperation) -> Result<()>;
    async fn load_mintop(&self, uid: Uuid) -> Result<MintOperation>;
    async fn list_mintops(&self, kid: cashu::Id) -> Result<Vec<MintOperation>>;
    async fn update_mintop(&self, uid: Uuid, minted: cashu::Amount) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait SignaturesRepository: Send + Sync {
    async fn store(&self, y: cashu::PublicKey, signature: cashu::BlindSignature) -> Result<()>;
    async fn load(&self, blind: &cashu::BlindedMessage) -> Result<Option<cashu::BlindSignature>>;
}

#[derive(Clone)]
pub struct Service {
    pub keys: Arc<dyn KeysRepository>,
    pub signatures: Arc<dyn SignaturesRepository>,
    pub keygen: Factory,
}

impl Service {
    pub async fn get_keyset_id_for_date(&self, date: TStamp) -> Result<cashu::Id> {
        let mut infos = self.keys.infos_for_expiration_date(date).await?;
        let tstamp = std::cmp::max(date.timestamp() as u64, 0);
        infos.retain(|info| info.final_expiry.unwrap_or_default() > tstamp);
        infos.sort_by_key(|info| info.final_expiry.expect("none is filtered out"));
        if !infos.is_empty() {
            return Ok(infos.first().expect("infos not empty").id);
        }
        let new_keyset = self.keygen.generate(date);
        let kid = new_keyset.0.id;
        self.keys.store(new_keyset).await?;
        Ok(kid)
    }

    pub async fn info(&self, kid: cashu::Id) -> Result<MintKeySetInfo> {
        self.keys.info(kid).await?.ok_or(Error::UnknownKeyset(kid))
    }

    pub async fn keys(&self, kid: cashu::Id) -> Result<cashu::MintKeySet> {
        self.keys
            .keyset(kid)
            .await?
            .ok_or(Error::UnknownKeyset(kid))
    }

    pub async fn verify_proof(&self, proof: cashu::Proof) -> Result<()> {
        let keyset = self.keys(proof.keyset_id).await?;
        keys_utils::verify_with_keys(&keyset, &proof)?;
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
            .ok_or(Error::UnknownKeyset(kid))?;
        info.active = false;
        self.keys.update_info(info.clone()).await?;
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
        let signature = keys_utils::sign_with_keys(&keyset, blind)?;
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
    ) -> Result<()> {
        let new = MintOperation {
            uid,
            kid,
            pub_key,
            target: amount,
            minted: cashu::Amount::ZERO,
        };
        self.keys.store_mintop(new).await?;
        Ok(())
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
            .unique()
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
        let operation = self.keys.load_mintop(request.quote).await?;
        let signature_verification = request.verify_signature(operation.pub_key);
        if signature_verification.is_err() {
            return Err(Error::InvalidMintRequest(String::from(
                "signature verifaction failed",
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
            let signature = keys_utils::sign_with_keys(&keyset, blind)?;
            self.signatures
                .store(blind.blinded_secret, signature.clone())
                .await?;
            signatures.push(signature);
        }
        self.keys
            .update_mintop(operation.uid, operation.minted + output_amount)
            .await?;
        let response = cashu::MintResponse { signatures };
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btc32::DerivationPath;
    use crate::test_utils::{TestKeysRepository, TestKeysService, TestSignaturesRepository};
    use bcr_wdc_utils::signatures::test_utils::generate_blinds;
    use cashu::Amount;
    use secp256k1::Keypair;
    use std::str::FromStr;

    // Helper function to set up test service
    async fn setup_test_service(
        amount: cashu::Amount,
    ) -> (TestKeysService, cashu::Id, Uuid, Keypair) {
        let seed = bip39::Mnemonic::from_str("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about")
            .unwrap()
            .to_seed("");
        let maturity = chrono::DateTime::parse_from_rfc3339("2029-01-01T00:00:00Z")
            .unwrap()
            .to_utc();
        let factory = Factory::new(&seed, DerivationPath::default());

        let service = TestKeysService {
            keys: Arc::new(TestKeysRepository::default()),
            signatures: Arc::new(TestSignaturesRepository::default()),
            keygen: factory,
        };
        let qid = Uuid::new_v4();
        let kp = bcr_wdc_utils::keys::test_utils::generate_random_keypair();
        let kid = service.get_keyset_id_for_date(maturity).await.unwrap();
        service
            .new_minting_operation(qid, kid, kp.public_key().into(), amount)
            .await
            .unwrap();
        (service, kid, qid, kp)
    }

    #[tokio::test]
    async fn test_mint_success() {
        let amounts = [Amount::from(128), Amount::from(64)];
        let total = amounts.iter().fold(Amount::ZERO, |acc, a| acc + *a);
        let (service, kid, qid, kp) = setup_test_service(total).await;
        let outputs = generate_blinds(kid, &amounts);
        let blinds = outputs.iter().map(|o| o.0.clone()).collect::<Vec<_>>();
        let mut request = cashu::MintRequest {
            quote: qid,
            outputs: blinds,
            signature: None,
        };
        request.sign(kp.secret_key().into()).unwrap();
        let signatures = service.mint(&request).await.unwrap().signatures;

        assert_eq!(signatures.len(), 2, "Should have 2 signatures");
        assert_eq!(
            signatures.iter().map(|s| u64::from(s.amount)).sum::<u64>(),
            192,
            "Total amount should match target"
        );
    }

    #[tokio::test]
    async fn test_mint_more() {
        let amounts = [Amount::from(128), Amount::from(64)];
        let total = amounts.iter().fold(Amount::ZERO, |acc, a| acc + *a);
        let (service, kid, qid, kp) = setup_test_service(total).await;
        let extra = [amounts[0], amounts[1], Amount::from(1)];
        let outputs = generate_blinds(kid, &extra);
        let blinds = outputs.iter().map(|o| o.0.clone()).collect::<Vec<_>>();
        let mut request = cashu::MintRequest {
            quote: qid,
            outputs: blinds,
            signature: None,
        };
        request.sign(kp.secret_key().into()).unwrap();

        assert!(
            service.mint(&request).await.is_err(),
            "Mint should fail with invalid amount"
        );
    }

    #[tokio::test]
    async fn test_mint_less() {
        let amounts = [Amount::from(128), Amount::from(32)];
        let total = amounts.iter().fold(Amount::ZERO, |acc, a| acc + *a);
        let (service, kid, qid, kp) = setup_test_service(total + total).await;
        let outputs = generate_blinds(kid, &amounts);
        let blinds = outputs.iter().map(|o| o.0.clone()).collect::<Vec<_>>();
        let mut request = cashu::MintRequest {
            quote: qid,
            outputs: vec![blinds[0].clone()],
            signature: None,
        };
        request.sign(kp.secret_key().into()).unwrap();
        service.mint(&request).await.unwrap();
        let mut request = cashu::MintRequest {
            quote: qid,
            outputs: vec![blinds[1].clone()],
            signature: None,
        };
        request.sign(kp.secret_key().into()).unwrap();
        service.mint(&request).await.unwrap();
    }
}
