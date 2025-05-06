// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_utils::keys as keys_utils;
use cashu::{nut00 as cdk00, nut01 as cdk01, nut02 as cdk02, Amount};
use cdk_common::mint::MintKeySetInfo;
use itertools::Itertools;
// ----- local imports
use crate::error::{Error, Result};
use crate::factory::Factory;
use crate::TStamp;

// ----- end imports

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[cfg_attr(test, derive(PartialEq))]
pub struct MintCondition {
    pub target: Amount,
    pub pub_key: cdk01::PublicKey,
    pub is_minted: bool,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeysRepository {
    async fn info(&self, id: &cdk02::Id) -> Result<Option<MintKeySetInfo>>;
    async fn list_info(&self) -> Result<Vec<MintKeySetInfo>>;
    async fn keyset(&self, id: &cdk02::Id) -> Result<Option<cdk02::MintKeySet>>;
    async fn list_keyset(&self) -> Result<Vec<cdk02::MintKeySet>>;
    async fn condition(&self, id: &cdk02::Id) -> Result<Option<MintCondition>>;
    async fn store(&self, keys: keys_utils::KeysetEntry, condition: MintCondition) -> Result<()>;
    // WARNING: it must fail if the keyset is already minted
    async fn mark_as_minted(&self, id: &cdk02::Id) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait QuoteKeysRepository {
    async fn entry(&self, qid: &uuid::Uuid) -> Result<Option<keys_utils::KeysetEntry>>;
    async fn info(&self, qid: &uuid::Uuid) -> Result<Option<MintKeySetInfo>>;
    async fn keyset(&self, qid: &uuid::Uuid) -> Result<Option<cdk02::MintKeySet>>;
    async fn condition(&self, qid: &uuid::Uuid) -> Result<Option<MintCondition>>;
    async fn store(
        &self,
        qid: &uuid::Uuid,
        keys: keys_utils::KeysetEntry,
        condition: MintCondition,
    ) -> Result<()>;
}

#[derive(Clone)]
pub struct Service<QuoteKeysRepo, KeysRepo> {
    pub quote_keys: QuoteKeysRepo,
    pub keys: KeysRepo,
    pub keygen: Factory,
}

impl<QuoteKeysRepo, KeysRepo> Service<QuoteKeysRepo, KeysRepo>
where
    KeysRepo: KeysRepository,
{
    pub async fn info(&self, kid: cdk02::Id) -> Result<MintKeySetInfo> {
        self.keys.info(&kid).await?.ok_or(Error::UnknownKeyset(kid))
    }

    pub async fn keys(&self, kid: cdk02::Id) -> Result<cdk02::MintKeySet> {
        self.keys
            .keyset(&kid)
            .await?
            .ok_or(Error::UnknownKeyset(kid))
    }

    pub async fn sign_blind(&self, blind: &cdk00::BlindedMessage) -> Result<cdk00::BlindSignature> {
        let keyset = self.keys(blind.keyset_id).await?;
        let signature = keys_utils::sign_with_keys(&keyset, blind)?;
        Ok(signature)
    }

    pub async fn verify_proof(&self, proof: cdk00::Proof) -> Result<()> {
        let keyset = self.keys(proof.keyset_id).await?;
        keys_utils::verify_with_keys(&keyset, &proof)?;
        Ok(())
    }

    pub async fn list_info(&self) -> Result<Vec<MintKeySetInfo>> {
        self.keys.list_info().await
    }

    pub async fn list_keyset(&self) -> Result<Vec<cdk02::MintKeySet>> {
        self.keys.list_keyset().await
    }
}

impl<QuoteKeysRepo, KeysRepo> Service<QuoteKeysRepo, KeysRepo>
where
    QuoteKeysRepo: QuoteKeysRepository,
{
    pub async fn pre_sign(
        &self,
        qid: uuid::Uuid,
        msg: &cdk00::BlindedMessage,
    ) -> Result<cdk00::BlindSignature> {
        let keyset = self
            .quote_keys
            .keyset(&qid)
            .await?
            .ok_or(Error::UnknownKeysetFromId(qid))?;
        let signature = keys_utils::sign_with_keys(&keyset, msg)?;
        Ok(signature)
    }

    pub async fn generate_keyset(
        &self,
        qid: uuid::Uuid,
        target: Amount,
        pub_key: cdk01::PublicKey,
        expire: TStamp,
    ) -> Result<cdk02::Id> {
        let mint_condition = MintCondition {
            target,
            pub_key,
            is_minted: false,
        };
        let info = self.quote_keys.info(&qid).await?;
        let id = match info {
            Some(info) => {
                let condition = self
                    .quote_keys
                    .condition(&qid)
                    .await?
                    .expect("info with not condition");
                if condition.pub_key != mint_condition.pub_key
                    || condition.target != mint_condition.target
                {
                    return Err(Error::InvalidGenerateRequest(qid));
                }
                info.id
            }
            None => {
                let keys_entry = self.keygen.generate(qid, expire);
                let id = keys_entry.1.id;
                self.quote_keys
                    .store(&qid, keys_entry, mint_condition)
                    .await?;
                id
            }
        };
        Ok(id)
    }
}

impl<QuoteKeysRepo, KeysRepo> Service<QuoteKeysRepo, KeysRepo>
where
    QuoteKeysRepo: QuoteKeysRepository,
    KeysRepo: KeysRepository,
{
    pub async fn activate(&self, qid: &uuid::Uuid) -> Result<()> {
        let mut entry = self
            .quote_keys
            .entry(qid)
            .await?
            .ok_or(Error::UnknownKeysetFromId(*qid))?;
        entry.0.active = true;
        let condition = self
            .quote_keys
            .condition(qid)
            .await?
            .ok_or(Error::UnknownKeysetFromId(*qid))?;

        self.keys.store(entry, condition).await?;
        Ok(())
    }
}

impl<QuoteKeysRepo, KeysRepo> Service<QuoteKeysRepo, KeysRepo>
where
    KeysRepo: KeysRepository,
{
    pub async fn authorized_public_key_to_mint(&self, kid: cdk02::Id) -> Result<cdk01::PublicKey> {
        let condition = self
            .keys
            .condition(&kid)
            .await?
            .ok_or(Error::UnknownKeyset(kid))?;
        Ok(condition.pub_key)
    }

    pub async fn mint(
        &self,
        _qid: uuid::Uuid,
        mut outputs: Vec<cdk00::BlindedMessage>,
    ) -> Result<Vec<cdk00::BlindSignature>> {
        //  check if the ids of the outputs are all the same
        let unique_ids: Vec<_> = outputs.iter().map(|p| p.keyset_id).unique().collect();
        if unique_ids.len() != 1 {
            return Err(Error::InvalidMintRequest);
        }
        let kid = unique_ids[0];

        //  check if the blinded secrets are unique
        let unique_secrets: Vec<_> = outputs.iter().map(|o| o.blinded_secret).unique().collect();
        if unique_secrets.len() != outputs.len() {
            return Err(Error::InvalidMintRequest);
        }

        let MintCondition {
            target, is_minted, ..
        } = self
            .keys
            .condition(&kid)
            .await?
            .ok_or(Error::UnknownKeyset(kid))?;
        //  check if the keyset id has been minted already
        if is_minted {
            return Err(Error::InvalidMintRequest);
        }

        let blinds = select_blinds_to_target(target, &mut outputs);
        let mut signatures = Vec::with_capacity(blinds.len());
        for blind in blinds {
            let signature = self.sign_blind(blind).await?;
            signatures.push(signature);
        }
        self.keys.mark_as_minted(&kid).await?;
        Ok(signatures)
    }
}

fn select_blinds_to_target(
    mut target: Amount,
    blinds: &mut [cdk00::BlindedMessage],
) -> &[cdk00::BlindedMessage] {
    for (idx, blind) in blinds.iter_mut().enumerate() {
        if target == Amount::ZERO {
            return &blinds[0..idx];
        }
        if blind.amount == Amount::ZERO {
            blind.amount = *target.split().first().expect("target > 0"); // split() returns from
                                                                         // highest to lowest
            target -= blind.amount;
        } else if blind.amount <= target {
            target -= blind.amount;
        } else {
            return &blinds[0..idx];
        }
    }
    blinds
}

#[cfg(test)]
pub mod tests {

    use super::*;
    use bcr_wdc_utils::keys::test_utils as keys_test;

    #[test]
    fn test_select_blind_signatures_no_valid_blinds() {
        let publics = keys_test::publics();
        let mut blinds = vec![
            cdk00::BlindedMessage {
                amount: Amount::from(16_u64),
                blinded_secret: publics[0],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: Amount::from(8_u64),
                blinded_secret: publics[1],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: Amount::from(32_u64),
                blinded_secret: publics[2],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
        ];
        let target = Amount::from(6_u64);
        let selected = select_blinds_to_target(target, &mut blinds);
        assert_eq!(selected.len(), 0);
    }

    #[test]
    fn test_select_blind_signatures_all_blanks() {
        let publics = keys_test::publics();
        let mut blinds = vec![
            cdk00::BlindedMessage {
                amount: Amount::from(0_u64),
                blinded_secret: publics[0],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: Amount::from(0_u64),
                blinded_secret: publics[1],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: Amount::from(0_u64),
                blinded_secret: publics[2],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
        ];
        let target = Amount::from(6_u64);
        let selected = select_blinds_to_target(target, &mut blinds);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].amount, Amount::from(4_u64));
        assert_eq!(selected[0].blinded_secret.to_hex(), keys_test::RANDOMS[0]);
        assert_eq!(selected[1].amount, Amount::from(2_u64));
        assert_eq!(selected[1].blinded_secret.to_hex(), keys_test::RANDOMS[1]);
    }

    #[test]
    fn test_select_blind_signatures_all_marked_blinds() {
        let publics = keys_test::publics();
        let mut blinds = vec![
            cdk00::BlindedMessage {
                amount: Amount::from(16_u64),
                blinded_secret: publics[1],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: Amount::from(4_u64),
                blinded_secret: publics[3],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: Amount::from(2_u64),
                blinded_secret: publics[2],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: Amount::from(1),
                blinded_secret: publics[0],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
        ];
        let target = Amount::from(6_u64);
        let selected = select_blinds_to_target(target, &mut blinds);
        assert_eq!(selected.len(), 0);
    }

    #[test]
    fn test_select_blind_signatures_marked_and_blanks() {
        let publics = keys_test::publics();
        let mut blinds = vec![
            cdk00::BlindedMessage {
                amount: Amount::from(4_u64),
                blinded_secret: publics[3],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: Amount::from(2_u64),
                blinded_secret: publics[2],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: Amount::from(0),
                blinded_secret: publics[0],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
        ];
        let target = Amount::from(6_u64);
        let selected = select_blinds_to_target(target, &mut blinds);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].amount, Amount::from(4_u64));
        assert_eq!(selected[0].blinded_secret.to_hex(), keys_test::RANDOMS[3]);
        assert_eq!(selected[1].amount, Amount::from(2_u64));
        assert_eq!(selected[1].blinded_secret.to_hex(), keys_test::RANDOMS[2]);
    }

    #[test]
    fn test_select_blind_signatures_unconventional_split() {
        let publics = keys_test::publics();
        let mut blinds = vec![
            cdk00::BlindedMessage {
                amount: Amount::from(4_u64),
                blinded_secret: publics[3],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: Amount::from(1),
                blinded_secret: publics[0],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: Amount::from(1_u64),
                blinded_secret: publics[1],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: Amount::from(0_u64),
                blinded_secret: publics[2],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
        ];
        let target = Amount::from(6_u64);
        let selected = select_blinds_to_target(target, &mut blinds);
        assert_eq!(selected.len(), 3);
        assert_eq!(selected[0].amount, Amount::from(4_u64));
        assert_eq!(selected[0].blinded_secret.to_hex(), keys_test::RANDOMS[3]);
        assert_eq!(selected[1].amount, Amount::from(1_u64));
        assert_eq!(selected[1].blinded_secret.to_hex(), keys_test::RANDOMS[0]);
        assert_eq!(selected[2].amount, Amount::from(1_u64));
        assert_eq!(selected[2].blinded_secret.to_hex(), keys_test::RANDOMS[1]);
    }
}
