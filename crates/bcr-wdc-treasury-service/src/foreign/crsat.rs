// ----- standard library imports
use std::{collections::HashSet, ops::Deref, str::FromStr, sync::Arc};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::wire::keys as wire_keys;
use bcr_wdc_utils::signatures::unblind_signatures;
use bitcoin::hashes::sha256::Hash as Sha256Hash;
// ----- local imports
use crate::{
    error::{Error, Result},
    foreign::proof,
    foreign::{
        fingerprints_vec_to_map, proofs_vec_to_map, ClowderClient, MintClientFactory,
        OfflineRepository, OnlineRepository,
    },
    TStamp,
};

// ----- end imports

#[async_trait]
pub trait KeysClient: proof::KeysClient {
    async fn get_keyset_with_expiration(
        &self,
        expiration: chrono::NaiveDate,
    ) -> Result<cashu::KeySet>;
}

pub struct Service {
    pub online_repo: Box<dyn OnlineRepository>,
    pub offline_repo: Box<dyn OfflineRepository>,
    pub keys: Box<dyn KeysClient>,
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
        let (foreign_mint_url, foreign_mint_pk) = self
            .clowder
            .can_accept_offline_exchange(inputs.clone())
            .await?;
        let foreign_fps = fingerprints_vec_to_map(inputs.clone(), hashes.clone());
        let mut retv: Vec<cashu::Proof> = Vec::new();
        for (kid, fps_hashes) in foreign_fps {
            let k_info = self.clowder.get_keyset_info(&foreign_mint_pk, &kid).await?;
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
            let proofs = unblind_signatures(premints.iter(), signatures.into_iter(), &keyset)?;
            retv.extend(proofs);
        }
        self.offline_repo
            .store_fps((foreign_mint_pk, foreign_mint_url), inputs, hashes)
            .await?;
        Ok(retv)
    }

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
        let kid = proofs[0].keyset_id;
        self.online_repo
            .store_htlc((*foreign_pk.deref(), foreign_mint), htlc_hash, proofs)
            .await?;
        let foreign_keyset = foreign_client.get_mint_keyset(kid).await?;
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
        let online_amount = try_online_htlc(
            preimage,
            self.online_repo.as_ref(),
            self.clowder.as_ref(),
            self.mint_factory.as_ref(),
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
) -> Result<cashu::Amount> {
    let mut gran_total = cashu::Amount::ZERO;
    let hash = Sha256Hash::from_str(preimage).map_err(|e| Error::InvalidInput(e.to_string()))?;
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
        // TODO: allow different keyset_ids
        assert_eq!(
            1,
            f_proofs
                .iter()
                .map(|p| p.keyset_id)
                .collect::<HashSet<_>>()
                .len(),
            "All foreign proofs must have the same keyset_id"
        );
        let foreign_client = factory.make_client(mint_url.clone()).await?;
        let keys = foreign_client
            .get_mint_keyset(f_proofs[0].keyset_id)
            .await?;
        let request = cashu::SwapRequest::new(f_proofs, premints.blinded_messages());
        let swap_response = foreign_client.post_swap(request).await?;
        let proofs =
            unblind_signatures(premints.iter(), swap_response.signatures.into_iter(), &keys)
                .map_err(|e| Error::Internal(e.to_string()))?;
        repo.store((mint_pk, mint_url), proofs).await?;
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
    let hash = Sha256Hash::from_str(preimage).map_err(|e| Error::InvalidInput(e.to_string()))?;
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
    repo.store_proofs((mint_pk, mint_url), vec![proof]).await?;
    Ok(amount)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_common::core_tests;
    use bitcoin::hex::prelude::*;
    use cashu::nut10 as cdk10;
    use mockall::predicate::*;

    mockall::mock! {
        pub KeysClient {}
        #[async_trait]
        impl proof::KeysClient for KeysClient {
            async fn sign(&self, blinds: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>>;
        }
        #[async_trait]
        impl KeysClient for KeysClient {
            async fn get_keyset_with_expiration(
                &self,
                expiration: chrono::NaiveDate,
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
        let secret = cdk10::Secret::from(conditions);
        let serialized = serde_json::to_vec(&secret).unwrap();
        let keypair = keyset.keys.get(&amount).expect("keys for amount");
        let (b_, r) =
            cashu::dhke::blind_message(&serialized, None).expect("cdk_dhke::blind_message");
        let c_ =
            cashu::dhke::sign_message(&keypair.secret_key, &b_).expect("cdk_dhke::sign_message");
        let c =
            cashu::dhke::unblind_message(&c_, &r, &keypair.public_key).expect("unblind_message");
        cashu::Proof::new(
            amount,
            keyset.id,
            cashu::secret::Secret::try_from(secret).unwrap(),
            c,
        )
    }

    #[tokio::test]
    async fn online_exchange_works() {
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
        let cloned_keyset = cashu::KeySet::from(foreign_keyset);
        factory
            .expect_make_client()
            .with(eq(foreign_url.clone()))
            .times(1)
            .returning(move |_| {
                let mut foreign_client =
                    bcr_common::client::cdk::test_utils::MockMintConnector::new();
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
                let cloned_keyset = cloned_keyset.clone();
                foreign_client
                    .expect_get_mint_keyset()
                    .with(eq(cloned_keyset.id))
                    .times(1)
                    .returning(move |_| Ok(cloned_keyset.clone()));
                Ok(Box::new(foreign_client))
            });
        clowder
            .expect_check_htlc_proofs()
            .with(eq(foreign_pk.clone()), eq(inputs.clone()))
            .times(1)
            .returning(|_, _| Ok(()));

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
            online_repo: Box::new(onlinerepo),
            offline_repo: Box::new(offlinerepo),
            keys: Box::new(keys),
            clowder: Arc::new(clowder),
            mint_factory: Arc::new(factory),
        };
        let proofs = srvc.online_exchange(inputs, &exchange_path).await.unwrap();
        assert_eq!(2, proofs.len());
    }
}
