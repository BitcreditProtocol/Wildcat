// ----- standard library imports
use std::{collections::HashMap, str::FromStr, sync::Arc};
// ----- extra library imports
use bcr_common::{cashu, core, wire::keys as wire_keys};
use bitcoin::{
    hashes::{sha256::Hash as Sha256Hash, Hash},
    secp256k1,
};
// ----- local imports
use crate::{
    error::{Error, Result},
    foreign::proof,
    foreign::{
        fingerprints_vec_to_map, to_mint_proofs_map, ClowderClient, KeysClient, MintClientFactory,
        OfflineRepository, OnlineRepository,
    },
    TStamp,
};

// ----- end imports

pub struct Service {
    pub online_repo: Arc<dyn OnlineRepository>,
    pub offline_repo: Arc<dyn OfflineRepository>,
    pub keys: Arc<dyn KeysClient>,
    pub clowder: Arc<dyn ClowderClient>,
    pub mint_factory: Arc<dyn MintClientFactory>,
}

impl Service {
    pub async fn offline_exchange(
        &self,
        inputs: Vec<wire_keys::ProofFingerprint>,
        hashes: Vec<Sha256Hash>,
        wpk: cashu::PublicKey,
    ) -> Result<Vec<cashu::Proof>> {
        let (_, foreign_mint_id) = self
            .clowder
            .can_accept_offline_exchange(inputs.clone())
            .await?;
        let foreign_fps = fingerprints_vec_to_map(inputs.clone(), hashes.clone());
        let mut retv: Vec<cashu::Proof> = Vec::new();
        for (kid, fps_hashes) in foreign_fps {
            let k_info = self.clowder.get_keyset_info(&foreign_mint_id, &kid).await?;
            let Some(foreign_unix_expiration) = k_info.final_expiry else {
                return Err(Error::InvalidInput(String::from(
                    "Foreign keyset has no expiration",
                )));
            };
            let foreign_expiration = TStamp::from_timestamp_secs(foreign_unix_expiration as i64)
                .ok_or(Error::InvalidInput(String::from(
                    "foreign expiry date parse",
                )))?;
            let foreign_date = foreign_expiration.date_naive();
            let keyset = self.keys.get_keyset_with_expiration(foreign_date).await?;
            let mut premints = cashu::PreMintSecrets::new(keyset.id);
            for (fp, hash) in fps_hashes {
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
                let proof = core::signature::unblind_ecash_signature(&keyset, pre.clone(), sig)?;
                proofs.push(proof);
            }
            retv.extend(proofs);
        }
        let result = self
            .clowder
            .signal_offline_exchange_event(inputs.clone(), hashes.clone(), wpk, retv.clone())
            .await;
        if let Err(e) = result {
            tracing::error!("clowder.signal_offline_exchange_event: {e}");
        }
        self.offline_repo
            .store_fps(foreign_mint_id, inputs, hashes)
            .await?;
        Ok(retv)
    }

    pub async fn online_exchange(
        &self,
        inputs: Vec<cashu::Proof>,
        path: Vec<secp256k1::PublicKey>,
    ) -> Result<Vec<cashu::Proof>> {
        if path.len() < 3 {
            return Err(Error::InvalidInput(String::from(
                "Exchange path must be at least [foreign pk, myself pk, wallet pk]",
            )));
        };
        let wallet_pk = path.last().unwrap();
        let myself_pk = path.get(path.len() - 2).unwrap();
        let foreign_pk = path.get(path.len() - 3).unwrap();
        let myself = self.clowder.get_myself_pk().await?;
        if &myself != myself_pk {
            return Err(Error::InvalidInput(String::from(
                "Exchange path must end with [myself pk, wallet pk]",
            )));
        };
        let foreign_mint = self.clowder.get_mint_url_from_pk(foreign_pk).await?;
        let foreign_client = self
            .mint_factory
            .make_client(foreign_mint, *foreign_pk)
            .await?;
        let (htlc_hash, foreign_locktime) = proof::check_htlc_foreign_proofs(
            *foreign_pk,
            &inputs,
            foreign_client.as_ref(),
            self.clowder.as_ref(),
        )
        .await?;
        let total = inputs
            .iter()
            .fold(cashu::Amount::ZERO, |total, p| total + p.amount);
        let kid = inputs[0].keyset_id;
        let foreign_keyset = foreign_client.get_keyset(kid).await?;
        let Some(foreign_unix_expiration) = foreign_keyset.final_expiry else {
            return Err(Error::InvalidInput(String::from(
                "Foreign keyset has no expiration",
            )));
        };
        let foreign_date = TStamp::from_timestamp_secs(foreign_unix_expiration as i64)
            .map(|tstamp| tstamp.date_naive());
        let Some(foreign_date) = foreign_date else {
            return Err(Error::InvalidInput(String::from(
                "foreign expiry date parse",
            )));
        };
        let keyset = self.keys.get_keyset_with_expiration(foreign_date).await?;
        let locktime = foreign_locktime - chrono::TimeDelta::minutes(15);
        let wallet_cpk = cashu::PublicKey::from(*wallet_pk);
        let outputs = proof::generate_htlc_proofs(
            total,
            Some(locktime),
            htlc_hash,
            wallet_cpk,
            &keyset,
            self.keys.as_ref(),
        )
        .await?;
        let proofs = self
            .clowder
            .signal_online_exchange_event(inputs.clone(), outputs.clone(), path.clone())
            .await?;
        let store_response = self
            .online_repo
            .store_htlc(*foreign_pk, htlc_hash, inputs)
            .await;
        if store_response.is_err() {
            tracing::error!(
                "failed to store_htlc, for {total} from {foreign_pk} with hash {htlc_hash}: {}",
                store_response.unwrap_err()
            );
        }
        Ok(proofs)
    }

    pub async fn try_swap_htlc(&self, preimage: &str, now: TStamp) -> Result<cashu::Amount> {
        let online_amount = try_online_htlc(
            preimage,
            self.online_repo.as_ref(),
            self.clowder.as_ref(),
            self.mint_factory.as_ref(),
            now,
        )
        .await?;
        if online_amount > cashu::Amount::ZERO {
            return Ok(online_amount);
        }
        let offline_amount =
            try_offline_htlc_swap(preimage, self.offline_repo.as_ref(), self.clowder.as_ref())
                .await?;
        Ok(offline_amount)
    }
}

async fn try_online_htlc(
    preimage: &str,
    repo: &dyn OnlineRepository,
    clowder: &dyn ClowderClient,
    factory: &dyn MintClientFactory,
    now: TStamp,
) -> Result<cashu::Amount> {
    let mut gran_total = cashu::Amount::ZERO;
    let hash = Sha256Hash::hash(preimage.as_bytes());
    let foreign_proofs = repo.search_htlc(&hash).await?;
    let foreign_proofs = to_mint_proofs_map(foreign_proofs);

    for (mint_id, mut f_proofs) in foreign_proofs {
        let mint_url = clowder.get_mint_url_from_pk(&mint_id).await?;
        let foreign_client = factory.make_client(mint_url, mint_id).await?;
        let kinfos = foreign_client.list_keyset_infos().await?;
        let swap_plan = core::swap::wallet::prepare_swap(&f_proofs, &kinfos)?;
        let mut outputs = Vec::with_capacity(f_proofs.len());
        let mut secrets = Vec::with_capacity(f_proofs.len());
        let mut keysets = HashMap::new();
        for (kid, amount) in swap_plan {
            let premint =
                cashu::PreMintSecrets::random(kid, amount, &cashu::amount::SplitTarget::None)?;
            outputs.extend(premint.blinded_messages());
            secrets.extend(premint.secrets);
            let keyset = foreign_client.get_keyset(kid).await?;
            keysets.insert(keyset.id, keyset);
        }
        let mut f_fingerprints = Vec::with_capacity(f_proofs.len());
        for proof in &mut f_proofs {
            proof.add_preimage(preimage.to_string());
            f_fingerprints.push(proof.y()?);
        }
        f_proofs = clowder.sign_p2pk_proofs(&f_proofs).await?;
        let signatures = foreign_client.swap(f_proofs, outputs, now).await?;
        let mut proofs = Vec::with_capacity(signatures.len());
        let mut total = cashu::Amount::ZERO;
        for (signature, secret) in signatures.into_iter().zip(secrets.iter()) {
            let keyset = keysets.get(&signature.keyset_id).unwrap();
            let proof =
                core::signature::unblind_ecash_signature(keyset, secret.clone(), signature)?;
            total += proof.amount;
            proofs.push(proof);
        }

        repo.store(mint_id, proofs).await?;
        repo.remove_htlcs(&f_fingerprints).await?;
        gran_total += total;
    }
    Ok(gran_total)
}

async fn try_offline_htlc_swap(
    preimage: &str,
    repo: &dyn OfflineRepository,
    clowder: &dyn ClowderClient,
) -> Result<cashu::Amount> {
    let hash = Sha256Hash::hash(preimage.as_bytes());
    let Some((mint_id, fp)) = repo.search_fp(&hash).await? else {
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
    let keys = clowder.get_keyset(&mint_id, &proof.keyset_id).await?;
    let key = keys
        .keys
        .get(&proof.amount)
        .ok_or(Error::Internal(String::from("key amount not found")))?;
    proof.verify_dleq(*key)?;
    repo.remove_fps(&[fp.y]).await?;
    repo.store_proofs(mint_id, vec![proof]).await?;
    Ok(amount)
}

#[cfg(test)]
mod tests {

    use super::*;
    use bcr_common::{core, core_tests};
    use bitcoin::hex::prelude::*;
    use mockall::predicate::*;

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
        let mut onlinerepo = crate::foreign::MockOnlineRepository::new();
        let offlinerepo = crate::foreign::MockOfflineRepository::new();
        let mut keys = crate::foreign::MockKeysClient::new();
        let mut clowder = crate::foreign::MockClowderClient::new();
        let mut factory = crate::foreign::MockMintClientFactory::new();
        let foreign_kp = core::generate_random_keypair();
        let myself_kp = core::generate_random_keypair();
        let wallet_kp = core::generate_random_keypair();
        let foreign_url = reqwest::Url::parse("https://foreign-mint.example").unwrap();
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
        let exchange_path = vec![
            foreign_kp.public_key(),
            myself_kp.public_key(),
            wallet_kp.public_key(),
        ];
        let myself_pk = myself_kp.public_key();
        let foreign_pk = foreign_kp.public_key();
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
        let cloned_keyset = cashu::KeySet::from(foreign_keyset);
        factory
            .expect_make_client()
            .with(eq(foreign_url.clone()), always())
            .times(1)
            .returning(move |_, _| {
                let mut foreign_client = crate::foreign::MockForeignClient::new();
                foreign_client
                    .expect_check_state()
                    .times(1)
                    .returning(|ys| {
                        Ok(vec![
                            cashu::ProofState {
                                y: ys[0],
                                state: cashu::State::Unspent,
                                witness: None,
                            },
                            cashu::ProofState {
                                y: ys[1],
                                state: cashu::State::Unspent,
                                witness: None,
                            },
                        ])
                    });
                let cloned_keyset = cloned_keyset.clone();
                foreign_client
                    .expect_get_keyset()
                    .with(eq(cloned_keyset.id))
                    .times(1)
                    .returning(move |_| Ok(cloned_keyset.clone()));
                Ok(Box::new(foreign_client))
            });
        clowder
            .expect_check_htlc_proofs()
            .with(eq(foreign_pk), eq(inputs.clone()))
            .times(1)
            .returning(|_, _| Ok(()));
        let cloned_inputs = inputs.clone();
        clowder
            .expect_signal_online_exchange_event()
            .times(1)
            .with(eq(inputs.clone()), always(), eq(exchange_path.clone()))
            .returning(move |_, _, _| Ok(cloned_inputs.clone()));
        let (_, mut myself_keyset) = core_tests::generate_random_ecash_keyset();
        myself_keyset.final_expiry = Some(expiration.timestamp() as u64);
        let cloned_keyset = cashu::KeySet::from(myself_keyset.clone());
        onlinerepo
            .expect_store_htlc()
            .times(1)
            .returning(|_, _, _| Ok(()));
        keys.expect_get_keyset_with_expiration()
            .with(eq(expiration.date_naive()))
            .times(1)
            .returning(move |_| Ok(cloned_keyset.clone()));
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
        };
        let proofs = srvc.online_exchange(inputs, exchange_path).await.unwrap();
        assert_eq!(2, proofs.len());
    }

    #[tokio::test]
    async fn offline_exchange_works() {
        let onlinerepo = crate::foreign::MockOnlineRepository::new();
        let mut offlinerepo = crate::foreign::MockOfflineRepository::new();
        let mut keys = crate::foreign::MockKeysClient::new();
        let mut clowder = crate::foreign::MockClowderClient::new();
        let factory = crate::foreign::MockMintClientFactory::new();
        let foreign_kp = core::generate_random_keypair();
        let myself_kp = core::generate_random_keypair();
        let wallet_kp = core::generate_random_keypair();
        let foreign_url = reqwest::Url::parse("https://foreign-mint.example").unwrap();
        let (mut foreign_info, foreign_keyset) = core_tests::generate_random_ecash_keyset();
        let expiration = chrono::Utc::now() + chrono::TimeDelta::days(7);
        foreign_info.final_expiry = Some(expiration.timestamp() as u64);
        let originals = [
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
        let foreign_kid = foreign_keyset.id;
        let foreign_pk = foreign_kp.public_key();
        let foreign_info = cashu::KeySetInfo::from(foreign_info);
        clowder
            .expect_get_keyset_info()
            .with(eq(foreign_pk), eq(foreign_kid))
            .times(1)
            .returning(move |_, _| Ok(foreign_info.clone()));
        let (_, mut myself_keyset) = core_tests::generate_random_ecash_keyset();
        myself_keyset.final_expiry = Some(expiration.timestamp() as u64);
        let cloned_keyset = cashu::KeySet::from(myself_keyset.clone());
        keys.expect_get_keyset_with_expiration()
            .with(eq(expiration.date_naive()))
            .times(1)
            .returning(move |_| Ok(cloned_keyset.clone()));
        let cloned_keyset = myself_keyset.clone();
        keys.expect_sign().times(1).returning(move |blinds| {
            let mut signatures = Vec::with_capacity(blinds.len());
            for blind in blinds {
                signatures
                    .push(bcr_common::core::signature::sign_ecash(&cloned_keyset, blind).unwrap());
            }
            Ok(signatures)
        });
        clowder
            .expect_signal_offline_exchange_event()
            .times(1)
            .with(
                eq(inputs.clone()),
                eq(hashes.clone()),
                eq(cashu::PublicKey::from(wallet_kp.public_key())),
                always(),
            )
            .returning(|_, _, _, _| Ok(()));
        offlinerepo
            .expect_store_fps()
            .with(eq(foreign_pk), eq(inputs.clone()), eq(hashes.clone()))
            .times(1)
            .returning(|_, _, _| Ok(()));

        let wallet_pk = cashu::PublicKey::from(wallet_kp.public_key());
        let srvc = Service {
            online_repo: Arc::new(onlinerepo),
            offline_repo: Arc::new(offlinerepo),
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
            mint_factory: Arc::new(factory),
        };
        let proofs = srvc
            .offline_exchange(inputs, hashes, wallet_pk)
            .await
            .unwrap();
        assert_eq!(2, proofs.len());
    }

    #[tokio::test]
    async fn try_swap_htlc_online() {
        let mut onlinerepo = crate::foreign::MockOnlineRepository::new();
        let offlinerepo = crate::foreign::MockOfflineRepository::new();
        let keys = crate::foreign::MockKeysClient::new();
        let mut clowder = crate::foreign::MockClowderClient::new();
        let mut factory = crate::foreign::MockMintClientFactory::new();
        let foreign_url = reqwest::Url::parse("https://foreign-mint.example").unwrap();
        let foreign_kp = core::generate_random_keypair();
        let wallet_kp = core::generate_random_keypair();
        let myself_kp = core::generate_random_keypair();
        let (foreign_kinfo, foreign_keyset) = core_tests::generate_random_ecash_keyset();
        let foreign_proof = generate_htlc_proof_for_online_exchange(
            &foreign_keyset,
            cashu::Amount::from(256),
            chrono::Utc::now() + chrono::TimeDelta::minutes(90),
            cashu::PublicKey::from(wallet_kp.public_key()),
            cashu::PublicKey::from(myself_kp.public_key()),
        );
        let preimage = foreign_proof.secret.to_string();
        let hash = Sha256Hash::hash(foreign_proof.secret.as_bytes());
        let search_response = vec![(foreign_kp.public_key(), foreign_proof.clone())];
        let myself_sk = cashu::SecretKey::from(myself_kp.secret_key());
        onlinerepo
            .expect_search_htlc()
            .with(eq(hash))
            .times(1)
            .returning(move |_| Ok(search_response.clone()));
        let cloned_url = foreign_url.clone();
        clowder
            .expect_get_mint_url_from_pk()
            .with(eq(foreign_kp.public_key()))
            .times(1)
            .returning(move |_| Ok(cloned_url.clone()));
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
            .with(eq(foreign_url.clone()), always())
            .times(1)
            .returning(move |_, _| {
                let cloned_keyset = cloned_keyset.clone();
                let mut foreign_client = crate::foreign::MockForeignClient::new();
                let keyset = cashu::KeySet::from(cloned_keyset.clone());
                let cloned_info = cashu::KeySetInfo::from(foreign_kinfo.clone());
                foreign_client
                    .expect_list_keyset_infos()
                    .times(1)
                    .returning(move || Ok(HashMap::from([(cloned_info.id, cloned_info.clone())])));
                foreign_client
                    .expect_get_keyset()
                    .with(eq(foreign_kid))
                    .times(1)
                    .returning(move |_| Ok(keyset.clone()));
                foreign_client
                    .expect_swap()
                    .times(1)
                    .returning(move |inputs, outputs, _| {
                        let mut signatures = Vec::with_capacity(inputs.len());
                        for blind in outputs {
                            let signature =
                                bcr_common::core::signature::sign_ecash(&cloned_keyset, &blind)
                                    .unwrap();
                            signatures.push(signature);
                        }
                        Ok(signatures)
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
        };
        let amount = srvc
            .try_swap_htlc(&preimage, chrono::Utc::now())
            .await
            .unwrap();
        assert_eq!(cashu::Amount::from(256), amount);
    }

    #[tokio::test]
    async fn try_swap_htlc_offline() {
        let mut onlinerepo = crate::foreign::MockOnlineRepository::new();
        let mut offlinerepo = crate::foreign::MockOfflineRepository::new();
        let keys = crate::foreign::MockKeysClient::new();
        let mut clowder = crate::foreign::MockClowderClient::new();
        let factory = crate::foreign::MockMintClientFactory::new();
        let foreign_kp = core::generate_random_keypair();
        let wallet_kp = core::generate_random_keypair();
        let myself_kp = core::generate_random_keypair();
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
            foreign_kp.public_key(),
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
            .with(eq(foreign_pk), always())
            .times(1)
            .returning(|_, _| Ok(()));
        let srvc = Service {
            online_repo: Arc::new(onlinerepo),
            offline_repo: Arc::new(offlinerepo),
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
            mint_factory: Arc::new(factory),
        };
        let amount = srvc
            .try_swap_htlc(&preimage, chrono::Utc::now())
            .await
            .unwrap();
        assert_eq!(cashu::Amount::from(256), amount);
    }
}
