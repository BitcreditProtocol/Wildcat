// ----- standard library imports
use std::{collections::HashSet, ops::Deref, str::FromStr, sync::Arc};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, core::signature::unblind_ecash_signature, wire::keys as wire_keys};
use bitcoin::hashes::{sha256::Hash as Sha256Hash, Hash};
// ----- local imports
use crate::{
    error::{Error, Result},
    foreign::{
        proof, proofs_vec_to_map, ClowderClient, MintClientFactory, OfflineRepository,
        OfflineSettleHandler, OnlineRepository,
    },
};

// ----- end imports

#[async_trait]
pub trait KeysClient: proof::KeysClient {
    async fn get_active_keyset(&self) -> Result<cashu::KeySet>;
}

pub struct Service {
    pub online_repo: Arc<dyn OnlineRepository>,
    pub offline_repo: Arc<dyn OfflineRepository>,
    pub keys: Arc<dyn KeysClient>,
    pub clowder: Arc<dyn ClowderClient>,
    pub mint_factory: Arc<dyn MintClientFactory>,
    pub settler: Box<dyn OfflineSettleHandler>,
}

impl Service {
    pub async fn offline_exchange(
        &self,
        inputs: Vec<wire_keys::ProofFingerprint>,
        hashes: Vec<Sha256Hash>,
        wpk: cashu::PublicKey,
    ) -> Result<Vec<cashu::Proof>> {
        let (foreign_mint_url, foreign_mint_pk) = self
            .clowder
            .can_accept_offline_exchange(inputs.clone())
            .await?;
        let keyset = self.keys.get_active_keyset().await?;
        let mut premints = cashu::PreMintSecrets::new(keyset.id);
        for (fp, hash) in inputs.iter().zip(hashes.iter()) {
            let condition = cashu::SpendingConditions::new_htlc_hash(
                &hash.to_string(),
                Some(cashu::Conditions {
                    pubkeys: Some(vec![wpk]),
                    ..Default::default()
                }),
            )?;
            let premint = cashu::PreMintSecrets::with_conditions(
                keyset.id,
                cashu::Amount::from(fp.amount),
                &cashu::amount::SplitTarget::None,
                &condition,
            )?;
            premints.combine(premint);
        }
        let signatures = self.keys.sign(&premints.blinded_messages()).await?;
        let mut proofs = Vec::with_capacity(signatures.len());
        for (sig, pre) in signatures.into_iter().zip(premints.iter()) {
            let proof = unblind_ecash_signature(&keyset, pre.clone(), sig)?;
            proofs.push(proof);
        }
        self.offline_repo
            .store_fps((foreign_mint_pk, foreign_mint_url), inputs, hashes)
            .await?;
        Ok(proofs)
    }

    const HTLC_BUFFER_MINUTES: i64 = 15;
    pub async fn online_exchange(
        &self,
        proofs: Vec<cashu::Proof>,
        path: &[cashu::PublicKey],
    ) -> Result<Vec<cashu::Proof>> {
        if path.len() < 3 {
            return Err(Error::InvalidInput(String::from(
                "Exchange path must be at least [foreign pk, myself pk, wallet pk]",
            )));
        };
        let mut path_it = path.iter().rev();
        let wallet_pk = path_it.next().unwrap();
        let myself = self.clowder.get_myself_pk().await?;
        let myself = cashu::PublicKey::from(myself.inner);
        if &myself != path_it.next().unwrap() {
            return Err(Error::InvalidInput(String::from(
                "Exchange path must end with [myself pk, wallet pk]",
            )));
        };
        let foreign_pk = path_it.next().unwrap();
        let foreign_mint = self.clowder.get_mint_url_from_pk(foreign_pk).await?;
        let foreign_client = self.mint_factory.make_client(foreign_mint.clone()).await?;
        let (htlc_hash, foreign_locktime) = proof::check_htlc_foreign_proofs(
            *foreign_pk,
            &proofs,
            foreign_client.as_ref(),
            self.clowder.as_ref(),
        )
        .await?;
        let total = proofs
            .iter()
            .fold(cashu::Amount::ZERO, |total, p| total + p.amount);
        self.online_repo
            .store_htlc((*foreign_pk.deref(), foreign_mint), htlc_hash, proofs)
            .await?;
        let keyset = self.keys.get_active_keyset().await?;
        let locktime = foreign_locktime - chrono::TimeDelta::minutes(Self::HTLC_BUFFER_MINUTES);
        let proofs = proof::generate_htlc_proofs(
            total,
            Some(locktime),
            htlc_hash,
            *wallet_pk,
            &keyset,
            self.keys.as_ref(),
        )
        .await?;
        Ok(proofs)
    }

    pub async fn try_swap_htlc(&self, preimage: &str) -> Result<cashu::Amount> {
        let online_amount = try_online_htlc_swap(
            preimage,
            self.online_repo.as_ref(),
            self.clowder.as_ref(),
            self.mint_factory.as_ref(),
        )
        .await?;
        if online_amount > cashu::Amount::ZERO {
            return Ok(online_amount);
        }
        let offline_amount = try_offline_htlc_swap(
            preimage,
            self.offline_repo.as_ref(),
            self.clowder.as_ref(),
            self.settler.as_ref(),
        )
        .await?;
        Ok(offline_amount)
    }

    pub async fn stop(&self) -> Result<()> {
        self.settler.stop().await
    }
}

async fn try_online_htlc_swap(
    preimage: &str,
    repo: &dyn OnlineRepository,
    clowder: &dyn ClowderClient,
    factory: &dyn MintClientFactory,
) -> Result<cashu::Amount> {
    let mut grand_total = cashu::Amount::ZERO;
    let hash = Sha256Hash::hash(preimage.as_bytes());
    let foreign_proofs = repo.search_htlc(&hash).await?;
    let foreign_proofs = proofs_vec_to_map(foreign_proofs);
    for ((mint_pk, mint_url), mut f_proofs) in foreign_proofs {
        let mut f_fingerprints = Vec::with_capacity(f_proofs.len());
        for proof in &mut f_proofs {
            proof.add_preimage(preimage.to_string());
            f_fingerprints.push(proof.y()?);
        }
        f_proofs = clowder.sign_p2pk_proofs(&f_proofs).await?;
        let total = f_proofs
            .iter()
            .fold(cashu::Amount::ZERO, |total, p| total + p.amount);
        let premints = cashu::PreMintSecrets::random(
            f_proofs[0].keyset_id,
            total,
            &cashu::amount::SplitTarget::None,
        )
        .map_err(|e| Error::Internal(e.to_string()))?;
        if f_proofs
            .iter()
            .map(|p| p.keyset_id)
            .collect::<HashSet<_>>()
            .len()
            != 1
        {
            return Err(Error::InvalidInput(
                "All foreign proofs must have the same keyset_id".to_string(),
            ));
        }
        let foreign_client = factory.make_client(mint_url.clone()).await?;
        let keys = foreign_client
            .get_mint_keyset(f_proofs[0].keyset_id)
            .await?;
        let request = cashu::SwapRequest::new(f_proofs, premints.blinded_messages());
        let swap_response = foreign_client.post_swap(request).await?;
        let mut proofs = Vec::with_capacity(swap_response.signatures.len());
        for (sig, pre) in swap_response.signatures.into_iter().zip(premints.iter()) {
            let proof = unblind_ecash_signature(&keys, pre.clone(), sig)?;
            proofs.push(proof);
        }
        repo.store((mint_pk, mint_url), proofs).await?;
        repo.remove_htlcs(&f_fingerprints).await?;
        grand_total += total;
    }
    Ok(grand_total)
}

async fn try_offline_htlc_swap(
    preimage: &str,
    repo: &dyn OfflineRepository,
    clowder: &dyn ClowderClient,
    settler: &dyn OfflineSettleHandler,
) -> Result<cashu::Amount> {
    let hash = Sha256Hash::hash(preimage.as_bytes());
    let Some(((mint_pk, mint_url), fp)) = repo.search_fp(&hash).await? else {
        return Ok(cashu::Amount::ZERO);
    };
    let secret = cashu::secret::Secret::from_str(preimage)?;
    let amount = cashu::Amount::from(fp.amount);
    let proof = cashu::Proof {
        amount,
        keyset_id: fp.keyset_id,
        c: fp.c,
        dleq: fp.dleq,
        witness: fp.witness,
        secret,
    };
    if proof.y()? != fp.y {
        return Err(Error::InvalidInput(String::from(
            "preimage does not match fingerprint",
        )));
    }
    let keys = clowder.get_keyset(&mint_pk, &proof.keyset_id).await?;
    let key = keys
        .keys
        .get(&proof.amount)
        .ok_or(Error::Internal(String::from("key amount not found")))?;
    proof.verify_dleq(*key)?;
    repo.remove_fps(&[fp.y]).await?;
    repo.store_proofs((mint_pk, mint_url.clone()), vec![proof])
        .await?;
    settler.monitor((mint_pk, mint_url))?;
    Ok(amount)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TStamp;
    use bcr_common::core_tests;
    use bitcoin::hex::prelude::*;
    use mockall::predicate::*;

    mockall::mock! {
        pub KeysClient {}
        #[async_trait]
        impl proof::KeysClient for KeysClient {
            async fn sign(&self, blinds: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>>;
        }
        #[async_trait]
        impl super::KeysClient for KeysClient {
            async fn get_active_keyset(
                &self,
            ) -> Result<cashu::KeySet>;
        }
    }

    fn generate_htlc_proof_for_online_exchange(
        keyset: &cashu::MintKeySet,
        amount: cashu::Amount,
        locktime: TStamp,
        wpk: cashu::PublicKey,
        mint: cashu::PublicKey,
    ) -> cashu::Proof {
        let preimage: [u8; 32] = rand::random();
        let conditions = cashu::SpendingConditions::new_htlc(
            format!("{:x}", preimage.as_hex()),
            Some(cashu::Conditions {
                locktime: Some(locktime.timestamp() as u64),
                pubkeys: Some(vec![mint]),
                refund_keys: Some(vec![wpk]),
                ..Default::default()
            }),
        )
        .unwrap();
        let premints = cashu::PreMintSecrets::with_conditions(
            keyset.id,
            amount,
            &cashu::amount::SplitTarget::None,
            &conditions,
        )
        .unwrap();
        assert_eq!(premints.blinded_messages().len(), 1);
        let blind = premints.blinded_messages()[0].clone();
        let signature = bcr_common::core::signature::sign_ecash(keyset, &blind).unwrap();
        bcr_common::core::signature::unblind_ecash_signature(
            &cashu::KeySet::from(keyset.clone()),
            premints.into_iter().next().unwrap(),
            signature,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn online_exchange_works() {
        let settler = crate::foreign::MockOfflineSettleHandler::new();
        let mut onlinerepo = crate::foreign::MockOnlineRepository::new();
        let offlinerepo = crate::foreign::MockOfflineRepository::new();
        let mut keys = MockKeysClient::new();
        let mut clowder = crate::foreign::tests::MockClowderClient::new();
        let mut factory = crate::foreign::MockMintClientFactory::new();
        let foreign_kp = core_tests::generate_random_keypair();
        let myself_kp = core_tests::generate_random_keypair();
        let wallet_kp = core_tests::generate_random_keypair();
        let foreign_url = cashu::MintUrl::from_str("https://foreign-mint.example").unwrap();
        let (_, mut foreign_keyset) = core_tests::generate_random_ecash_keyset();
        let expiration = chrono::Utc::now() + chrono::TimeDelta::days(7);
        foreign_keyset.final_expiry = Some(expiration.timestamp() as u64);
        let inputs = vec![
            generate_htlc_proof_for_online_exchange(
                &foreign_keyset,
                cashu::Amount::from(512),
                chrono::Utc::now() + chrono::TimeDelta::minutes(90),
                cashu::PublicKey::from(wallet_kp.public_key()),
                cashu::PublicKey::from(myself_kp.public_key()),
            ),
            generate_htlc_proof_for_online_exchange(
                &foreign_keyset,
                cashu::Amount::from(256),
                chrono::Utc::now() + chrono::TimeDelta::minutes(90),
                cashu::PublicKey::from(wallet_kp.public_key()),
                cashu::PublicKey::from(myself_kp.public_key()),
            ),
        ];
        let exchange_path = [
            cashu::PublicKey::from(foreign_kp.public_key()),
            cashu::PublicKey::from(myself_kp.public_key()),
            cashu::PublicKey::from(wallet_kp.public_key()),
        ];
        let myself_pk = myself_kp.public_key();
        let foreign_pk = cashu::PublicKey::from(foreign_kp.public_key());
        clowder
            .expect_get_myself_pk()
            .times(1)
            .returning(move || Ok(myself_pk.into()));
        let cloned_url = foreign_url.clone();
        clowder
            .expect_get_mint_url_from_pk()
            .with(eq(foreign_pk))
            .times(1)
            .returning(move |_| Ok(cloned_url.clone()));
        factory
            .expect_make_client()
            .with(eq(foreign_url.clone()))
            .times(1)
            .returning(move |_| {
                let mut foreign_client = crate::foreign::test_utils::MockMintConnector::new();
                foreign_client
                    .expect_post_check_state()
                    .times(1)
                    .returning(|request| {
                        Ok(cashu::CheckStateResponse {
                            states: vec![
                                cashu::ProofState {
                                    y: request.ys[0],
                                    state: cashu::State::Unspent,
                                    witness: None,
                                },
                                cashu::ProofState {
                                    y: request.ys[1],
                                    state: cashu::State::Unspent,
                                    witness: None,
                                },
                            ],
                        })
                    });
                Ok(Box::new(foreign_client))
            });
        clowder
            .expect_check_htlc_proofs()
            .with(eq(foreign_pk), eq(inputs.clone()))
            .times(1)
            .returning(|_, _| Ok(()));
        let (_, mut myself_keyset) = core_tests::generate_random_ecash_keyset();
        myself_keyset.final_expiry = Some(expiration.timestamp() as u64);
        let cloned_keyset = cashu::KeySet::from(myself_keyset.clone());
        onlinerepo
            .expect_store_htlc()
            .times(1)
            .returning(|_, _, _| Ok(()));
        keys.expect_get_active_keyset()
            .times(1)
            .returning(move || Ok(cloned_keyset.clone()));
        let cloned_keyset = myself_keyset.clone();
        keys.expect_sign().times(1).returning(move |blinds| {
            let mut signatures = Vec::with_capacity(blinds.len());
            for blind in blinds {
                signatures
                    .push(bcr_common::core::signature::sign_ecash(&cloned_keyset, blind).unwrap());
            }
            Ok(signatures)
        });
        let srvc = Service {
            online_repo: Arc::new(onlinerepo),
            offline_repo: Arc::new(offlinerepo),
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
            mint_factory: Arc::new(factory),
            settler: Box::new(settler),
        };
        let proofs = srvc.online_exchange(inputs, &exchange_path).await.unwrap();
        assert_eq!(2, proofs.len());
    }

    #[tokio::test]
    async fn offline_exchange_works() {
        let settler = crate::foreign::MockOfflineSettleHandler::new();
        let onlinerepo = crate::foreign::MockOnlineRepository::new();
        let mut offlinerepo = crate::foreign::MockOfflineRepository::new();
        let mut keys = MockKeysClient::new();
        let mut clowder = crate::foreign::tests::MockClowderClient::new();
        let factory = crate::foreign::MockMintClientFactory::new();
        let foreign_kp = core_tests::generate_random_keypair();
        let myself_kp = core_tests::generate_random_keypair();
        let wallet_kp = core_tests::generate_random_keypair();
        let foreign_url = reqwest::Url::parse("https://foreign-mint.example").unwrap();
        let (mut foreign_info, foreign_keyset) = core_tests::generate_random_ecash_keyset();
        let expiration = chrono::Utc::now() + chrono::TimeDelta::days(7);
        foreign_info.final_expiry = Some(expiration.timestamp() as u64);
        let originals = vec![
            generate_htlc_proof_for_online_exchange(
                &foreign_keyset,
                cashu::Amount::from(512),
                chrono::Utc::now() + chrono::TimeDelta::minutes(90),
                cashu::PublicKey::from(wallet_kp.public_key()),
                cashu::PublicKey::from(myself_kp.public_key()),
            ),
            generate_htlc_proof_for_online_exchange(
                &foreign_keyset,
                cashu::Amount::from(256),
                chrono::Utc::now() + chrono::TimeDelta::minutes(90),
                cashu::PublicKey::from(wallet_kp.public_key()),
                cashu::PublicKey::from(myself_kp.public_key()),
            ),
        ];
        let inputs = originals
            .iter()
            .map(|p| wire_keys::ProofFingerprint::try_from(p.clone()).unwrap())
            .collect::<Vec<_>>();
        let hashes = originals
            .iter()
            .map(|p| Sha256Hash::hash(p.secret.as_bytes()))
            .collect::<Vec<_>>();
        let cloned_url = foreign_url.clone();
        let foreign_pk = foreign_kp.public_key();
        clowder
            .expect_can_accept_offline_exchange()
            .times(1)
            .with(eq(inputs.clone()))
            .returning(move |_| Ok((cloned_url.clone(), foreign_pk)));
        let foreign_pk = foreign_kp.public_key();
        let (_, mut myself_keyset) = core_tests::generate_random_ecash_keyset();
        myself_keyset.final_expiry = Some(expiration.timestamp() as u64);
        let cloned_keyset = cashu::KeySet::from(myself_keyset.clone());
        keys.expect_get_active_keyset()
            .times(1)
            .returning(move || Ok(cloned_keyset.clone()));
        let cloned_keyset = myself_keyset.clone();
        keys.expect_sign().times(1).returning(move |blinds| {
            let mut signatures = Vec::with_capacity(blinds.len());
            for blind in blinds {
                signatures
                    .push(bcr_common::core::signature::sign_ecash(&cloned_keyset, blind).unwrap());
            }
            Ok(signatures)
        });
        offlinerepo
            .expect_store_fps()
            .with(
                eq((foreign_pk, foreign_url.clone())),
                eq(inputs.clone()),
                eq(hashes.clone()),
            )
            .times(1)
            .returning(|_, _, _| Ok(()));
        let wallet_pk = cashu::PublicKey::from(wallet_kp.public_key());
        let srvc = Service {
            online_repo: Arc::new(onlinerepo),
            offline_repo: Arc::new(offlinerepo),
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
            mint_factory: Arc::new(factory),
            settler: Box::new(settler),
        };
        let proofs = srvc
            .offline_exchange(inputs, hashes, wallet_pk)
            .await
            .unwrap();
        assert_eq!(2, proofs.len());
    }

    #[tokio::test]
    async fn try_swap_htlc_online() {
        let settler = crate::foreign::MockOfflineSettleHandler::new();
        let mut onlinerepo = crate::foreign::MockOnlineRepository::new();
        let offlinerepo = crate::foreign::MockOfflineRepository::new();
        let keys = MockKeysClient::new();
        let mut clowder = crate::foreign::tests::MockClowderClient::new();
        let mut factory = crate::foreign::MockMintClientFactory::new();
        let foreign_url = cashu::MintUrl::from_str("https://foreign-mint.example").unwrap();
        let foreign_kp = core_tests::generate_random_keypair();
        let wallet_kp = core_tests::generate_random_keypair();
        let myself_kp = core_tests::generate_random_keypair();
        let (_, foreign_keyset) = core_tests::generate_random_ecash_keyset();
        let foreign_proof = generate_htlc_proof_for_online_exchange(
            &foreign_keyset,
            cashu::Amount::from(256),
            chrono::Utc::now() + chrono::TimeDelta::minutes(90),
            cashu::PublicKey::from(wallet_kp.public_key()),
            cashu::PublicKey::from(myself_kp.public_key()),
        );
        let preimage = foreign_proof.secret.to_string();
        let hash = Sha256Hash::hash(foreign_proof.secret.as_bytes());
        let search_response = vec![(
            (foreign_kp.public_key(), foreign_url.clone()),
            foreign_proof.clone(),
        )];
        let myself_sk = cashu::SecretKey::from(myself_kp.secret_key());
        onlinerepo
            .expect_search_htlc()
            .with(eq(hash))
            .times(1)
            .returning(move |_| Ok(search_response.clone()));
        clowder
            .expect_sign_p2pk_proofs()
            .times(1)
            .returning(move |proofs| {
                let mut proofs = proofs.to_vec();
                proofs
                    .iter_mut()
                    .for_each(|p| p.sign_p2pk(myself_sk.clone()).unwrap());
                Ok(proofs)
            });
        let foreign_kid = foreign_keyset.id;
        let cloned_keyset = foreign_keyset.clone();
        factory
            .expect_make_client()
            .with(eq(foreign_url.clone()))
            .times(1)
            .returning(move |_| {
                let cloned_keyset = cloned_keyset.clone();
                let mut foreign_client = crate::foreign::test_utils::MockMintConnector::new();
                let keyset = cashu::KeySet::from(cloned_keyset.clone());
                foreign_client
                    .expect_get_mint_keyset()
                    .with(eq(foreign_kid))
                    .times(1)
                    .returning(move |_| Ok(keyset.clone()));
                foreign_client
                    .expect_post_swap()
                    .times(1)
                    .returning(move |request| {
                        let mut signatures = Vec::with_capacity(request.inputs().len());
                        for blind in request.outputs() {
                            let signature =
                                bcr_common::core::signature::sign_ecash(&cloned_keyset, blind)
                                    .unwrap();
                            signatures.push(signature);
                        }
                        Ok(cashu::SwapResponse { signatures })
                    });
                Ok(Box::new(foreign_client))
            });
        onlinerepo.expect_store().times(1).returning(|_, _| Ok(()));
        let foreign_y = foreign_proof.y().unwrap();
        onlinerepo
            .expect_remove_htlcs()
            .with(eq(vec![foreign_y]))
            .times(1)
            .returning(|_| Ok(()));
        let srvc = Service {
            online_repo: Arc::new(onlinerepo),
            offline_repo: Arc::new(offlinerepo),
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
            mint_factory: Arc::new(factory),
            settler: Box::new(settler),
        };
        let amount = srvc.try_swap_htlc(&preimage).await.unwrap();
        assert_eq!(cashu::Amount::from(256), amount);
    }

    #[tokio::test]
    async fn try_swap_htlc_offline() {
        let mut settler = crate::foreign::MockOfflineSettleHandler::new();
        let mut onlinerepo = crate::foreign::MockOnlineRepository::new();
        let mut offlinerepo = crate::foreign::MockOfflineRepository::new();
        let keys = MockKeysClient::new();
        let mut clowder = crate::foreign::tests::MockClowderClient::new();
        let factory = crate::foreign::MockMintClientFactory::new();
        let foreign_url = reqwest::Url::parse("https://foreign-mint.example").unwrap();
        let foreign_kp = core_tests::generate_random_keypair();
        let wallet_kp = core_tests::generate_random_keypair();
        let myself_kp = core_tests::generate_random_keypair();
        let (_, foreign_keyset) = core_tests::generate_random_ecash_keyset();
        let foreign_proof = generate_htlc_proof_for_online_exchange(
            &foreign_keyset,
            cashu::Amount::from(256),
            chrono::Utc::now() + chrono::TimeDelta::minutes(90),
            cashu::PublicKey::from(wallet_kp.public_key()),
            cashu::PublicKey::from(myself_kp.public_key()),
        );
        let preimage = foreign_proof.secret.to_string();
        let hash = Sha256Hash::hash(foreign_proof.secret.as_bytes());
        let search_response = (
            (foreign_kp.public_key(), foreign_url.clone()),
            bcr_common::wire::keys::ProofFingerprint::try_from(foreign_proof.clone()).unwrap(),
        );
        onlinerepo
            .expect_search_htlc()
            .with(eq(hash))
            .times(1)
            .returning(move |_| Ok(vec![]));
        offlinerepo
            .expect_search_fp()
            .with(eq(hash))
            .times(1)
            .returning(move |_| Ok(Some(search_response.clone())));
        let foreign_kid = foreign_keyset.id;
        let foreign_pk = foreign_kp.public_key();
        let cloned_keyset = cashu::KeySet::from(foreign_keyset);
        clowder
            .expect_get_keyset()
            .with(eq(foreign_pk), eq(foreign_kid))
            .times(1)
            .returning(move |_, _| Ok(cloned_keyset.clone()));
        let foreign_y = foreign_proof.y().unwrap();
        offlinerepo
            .expect_remove_fps()
            .with(eq(vec![foreign_y]))
            .times(1)
            .returning(|_| Ok(()));
        offlinerepo
            .expect_store_proofs()
            .with(eq((foreign_pk, foreign_url.clone())), always())
            .times(1)
            .returning(|_, _| Ok(()));
        settler
            .expect_monitor()
            .with(eq((foreign_pk, foreign_url)))
            .times(1)
            .returning(|_| Ok(()));
        let srvc = Service {
            online_repo: Arc::new(onlinerepo),
            offline_repo: Arc::new(offlinerepo),
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
            mint_factory: Arc::new(factory),
            settler: Box::new(settler),
        };
        let amount = srvc.try_swap_htlc(&preimage).await.unwrap();
        assert_eq!(cashu::Amount::from(256), amount);
    }
}
