// ----- standard library imports
use std::{collections::HashMap, str::FromStr, sync::Arc};
// ----- extra library imports
use bcr_common::{
    cashu::{self, ProofsMethods},
    client::{admin::treasury::SUError, clowder::ClowderClientError},
    core::{
        self,
        htlc::{exchange_htlc, hop_locktime, htlc_lock, offline_hash_lock, online_hash_lock},
    },
    wire::{
        exchange::{exchange_digest, exchange_message, OfflineExchangeRequest},
        keys as wire_keys,
    },
};
use bitcoin::{hashes::sha256::Hash as Sha256Hash, secp256k1};
// ----- local imports
use crate::{
    error::{Error, Result},
    foreign::{
        fingerprints_vec_to_map, proof, signed_swap_with_foreign, to_mint_proofs_map,
        ClowderClient, KeysClient, MintBalance, MintClientFactory, OfflineRepository,
        OfflineReservation, OnlineRepository, ReservationState,
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
    pub exchange_lock_margin_secs: u64,
    pub offline_exchange_lock_secs: u64,
}

impl Service {
    pub async fn offline_exchange(
        &self,
        inputs: Vec<wire_keys::ProofFingerprint>,
        hashes: Vec<Sha256Hash>,
        wpk: cashu::PublicKey,
        wallet_signature: secp256k1::schnorr::Signature,
        now: TStamp,
    ) -> Result<Vec<cashu::Proof>> {
        let request = OfflineExchangeRequest {
            fingerprints: inputs.clone(),
            hashes: hashes.clone(),
            wallet_pk: wpk,
            wallet_signature,
        };
        let ys: Vec<cashu::PublicKey> = inputs.iter().map(|fp| fp.y).collect();
        if let Some(reservation) = self.offline_repo.search_reservation(&ys).await? {
            return self.replay_offline_exchange(reservation, request).await;
        }
        let (_, foreign_mint_id) = self
            .clowder
            .can_accept_offline_exchange(inputs.clone())
            .await?;
        // Expires to its issuer; a wallet that never unlocks redeems at its own alpha.
        let refund = cashu::PublicKey::from(self.clowder.get_myself_pk().await?);
        let expires_at = now + time::Duration::seconds(self.offline_exchange_lock_secs as i64);
        let locktime = expires_at.unix_timestamp() as u64;
        // No offline eCash is issued until the node has verified the wallet's
        // signature, recorded the dual-signed exchange and broadcast the
        // evidence to the alpha's Betas.
        let recorded = self.clowder.record_offline_exchange(&request).await?;
        let digest = exchange_digest(
            &foreign_mint_id,
            &recorded.evidence_digest,
            &inputs,
            &hashes,
            &wpk,
        );
        if digest != recorded.exchange_digest {
            return Err(Error::InvalidInput(String::from(
                "recorded exchange digest mismatch",
            )));
        }
        let foreign_fps = fingerprints_vec_to_map(inputs.clone(), hashes.clone());
        let mut batches = Vec::new();
        for (kid, fps_hashes) in foreign_fps {
            let k_info = self.clowder.get_keyset_info(&foreign_mint_id, &kid).await?;
            let Some(foreign_unix_expiration) = k_info.final_expiry else {
                return Err(Error::InvalidInput(String::from(
                    "Foreign keyset has no expiration",
                )));
            };
            let foreign_expiration = TStamp::from_unix_timestamp(foreign_unix_expiration as i64)
                .map_err(|_| Error::InvalidInput(String::from("foreign expiry date parse")))?;
            let foreign_date = foreign_expiration.date();
            let keyset = self.keys.get_keyset_with_expiration(foreign_date).await?;
            let mut secrets = Vec::new();
            for (fp, hash) in fps_hashes {
                let amount = cashu::Amount::from(fp.amount);
                let condition = exchange_htlc(hash, locktime, wpk, refund)?;
                // Tag the secret so it's verified by the raw-bytes offline verifier.
                for part in amount
                    .split_targeted(
                        &cashu::amount::SplitTarget::None,
                        &core::keys::to_fee_and_amounts(&keyset),
                    )
                    .map_err(|e| Error::InvalidInput(e.to_string()))?
                {
                    let secret = core::signature::offline_htlc_secret(condition.clone())?;
                    let (blinded, r) = cashu::dhke::blind_message(&secret.to_bytes(), None)?;
                    secrets.push(cashu::PreMint {
                        secret,
                        blinded_message: cashu::BlindedMessage::new(
                            part,
                            keyset.id.into(),
                            blinded,
                        ),
                        r,
                        amount: part,
                    });
                }
            }
            let premints = cashu::PreMintSecrets {
                secrets,
                keyset_id: keyset.id.into(),
            };
            batches.push((keyset, premints));
        }
        let reservation = OfflineReservation {
            exchange_digest: digest,
            alpha_id: foreign_mint_id,
            evidence_digest: recorded.evidence_digest,
            state: ReservationState::Reserved,
        };
        if !self.offline_repo.reserve_exchange(reservation, &ys).await? {
            return Err(exchange_in_progress());
        }
        let mut retv: Vec<cashu::Proof> = Vec::new();
        for (keyset, premints) in batches {
            let signatures = self.keys.sign(&premints.blinded_messages()).await?;
            for (sig, pre) in signatures.into_iter().zip(premints.iter()) {
                retv.push(core::signature::unblind_ecash_signature(
                    &keyset,
                    pre.clone(),
                    sig,
                )?);
            }
        }
        if !self
            .offline_repo
            .issue_reservation(
                digest,
                foreign_mint_id,
                inputs,
                hashes,
                retv.clone(),
                expires_at,
            )
            .await?
        {
            return Err(exchange_in_progress());
        }
        self.announce_offline_exchange(request, digest, retv, expires_at)
            .await
    }

    async fn replay_offline_exchange(
        &self,
        reservation: OfflineReservation,
        request: OfflineExchangeRequest,
    ) -> Result<Vec<cashu::Proof>> {
        let digest = exchange_digest(
            &reservation.alpha_id,
            &reservation.evidence_digest,
            &request.fingerprints,
            &request.hashes,
            &request.wallet_pk,
        );
        if digest != reservation.exchange_digest {
            return Err(Error::InvalidInput(String::from(
                "inputs held by another exchange",
            )));
        }
        secp256k1::global::SECP256K1
            .verify_schnorr(
                &request.wallet_signature,
                &exchange_message(&digest),
                &request.wallet_pk.x_only_public_key(),
            )
            .map_err(|_| Error::InvalidInput(String::from("invalid wallet signature")))?;
        match reservation.state {
            ReservationState::Reserved => Err(exchange_in_progress()),
            ReservationState::Issued { proofs, expires_at } => {
                self.announce_offline_exchange(request, digest, proofs, expires_at)
                    .await
            }
            ReservationState::Complete(proofs) => Ok(proofs),
        }
    }

    /// Signals and records issued proofs, then marks the exchange complete; a replay of an
    /// exchange that stopped part way runs this again with the stored proofs. A failure
    /// leaves it Issued, so no proofs leave unannounced; it is retryable unless the node
    /// refused the signal for good, which is passed through.
    async fn announce_offline_exchange(
        &self,
        request: OfflineExchangeRequest,
        digest: [u8; 32],
        proofs: Vec<cashu::Proof>,
        expires_at: TStamp,
    ) -> Result<Vec<cashu::Proof>> {
        match self
            .try_announce_offline_exchange(request, digest, &proofs, expires_at)
            .await
        {
            Ok(()) => Ok(proofs),
            Err(Error::ClowderNatsClient(ClowderClientError::Rejected(r))) if !r.is_transient() => {
                tracing::warn!("offline exchange left issued, refused: {r}");
                Err(Error::ClowderNatsClient(ClowderClientError::Rejected(r)))
            }
            Err(e) => {
                tracing::warn!("offline exchange left issued, retryable: {e}");
                Err(exchange_in_progress())
            }
        }
    }

    async fn try_announce_offline_exchange(
        &self,
        request: OfflineExchangeRequest,
        digest: [u8; 32],
        proofs: &[cashu::Proof],
        expires_at: TStamp,
    ) -> Result<()> {
        self.clowder
            .signal_offline_exchange_event(
                request.fingerprints,
                request.hashes,
                request.wallet_pk,
                proofs.to_vec(),
                Some(digest),
                Some(request.wallet_signature),
            )
            .await?;
        let mut issued: HashMap<Sha256Hash, Vec<cashu::Proof>> = HashMap::new();
        for proof in proofs {
            let (hash, _) = htlc_lock(proof)?;
            issued.entry(hash).or_default().push(proof.clone());
        }
        for (hash, hash_proofs) in issued {
            self.online_repo
                .store_issued(hash, expires_at, hash_proofs)
                .await?;
        }
        self.offline_repo.complete_reservation(digest).await
    }

    pub async fn online_exchange(
        &self,
        inputs: Vec<cashu::Proof>,
        path: Vec<secp256k1::PublicKey>,
        now: TStamp,
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
        let locktime = hop_locktime(
            foreign_locktime,
            now.unix_timestamp() as u64,
            self.exchange_lock_margin_secs,
        )
        .ok_or(Error::InvalidInput(String::from(
            "foreign lock leaves no hop margin",
        )))?;
        let wallet_cpk = cashu::PublicKey::from(*wallet_pk);
        // Only this mint may reclaim once the locktime passes.
        let refund = cashu::PublicKey::from(myself);
        let outputs = proof::generate_online_exchange_htlc_proofs(
            &inputs,
            locktime,
            htlc_hash,
            wallet_cpk,
            refund,
            *foreign_pk,
            self.clowder.as_ref(),
            self.keys.as_ref(),
        )
        .await?;
        let proofs = self
            .clowder
            .signal_online_exchange_event(inputs.clone(), outputs.clone(), path.clone())
            .await?;
        self.online_repo
            .store_htlc(*foreign_pk, htlc_hash, inputs)
            .await?;
        // Kept so an issuance the recipient never unlocks can be reclaimed at its
        // locktime instead of circulating unbacked.
        let locktime = TStamp::from_unix_timestamp(locktime as i64)
            .map_err(|_| Error::InvalidInput(String::from("invalid HTLC time tag")))?;
        self.online_repo
            .store_issued(htlc_hash, locktime, outputs)
            .await?;
        Ok(proofs)
    }

    /// Claims an exchange the alpha spent at recovery but never issued against.
    /// The node authorises against the spend entry it recorded, so this only signs
    /// outputs a spend entry already owes.
    pub async fn redeem_offline_exchange(
        &self,
        request: bcr_common::wire::exchange::RedeemOfflineExchangeRequest,
    ) -> Result<Vec<cashu::BlindSignature>> {
        let authorized = self.clowder.redeem_offline_exchange(&request).await?;
        // Before signing: a withheld blind signature is still fetchable from restore.
        // Before claiming: a bad request must not spend the entry's one issuance.
        let claimed = cashu::Amount::try_sum(request.outputs.iter().map(|o| o.amount))
            .map_err(|_| Error::InvalidInput(String::from("redemption amount overflow")))?;
        if claimed != authorized.amount {
            return Err(Error::InvalidInput(String::from(
                "outputs ask for more than the spend entry authorised",
            )));
        }
        // One entry, one issuance, however often the claim is replayed.
        if !self
            .offline_repo
            .claim_redemption(request.exchange_digest)
            .await?
        {
            return Err(Error::InvalidInput(String::from(
                "spend entry already redeemed",
            )));
        }
        let signatures = self.keys.sign(&request.outputs).await?;
        self.clowder
            .signal_offline_redeem_event(request, signatures.clone())
            .await?;
        Ok(signatures)
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
        let offline_amount = try_offline_htlc_swap(
            preimage,
            self.offline_repo.as_ref(),
            self.online_repo.as_ref(),
            self.clowder.as_ref(),
        )
        .await?;
        Ok(offline_amount)
    }

    /// Foreign eCash held per issuing mint: swapped and owned outright, and held for
    /// a mint that is still offline. A mint shows up if either figure is non-zero.
    pub async fn balance(&self) -> Result<Vec<MintBalance>> {
        let settled = self.online_repo.settled_balance().await?;
        let mut unsettled = self.offline_repo.unsettled_balance().await?;
        let mut balances = Vec::with_capacity(settled.len() + unsettled.len());
        for (mint_id, settled) in settled {
            // Taking it out leaves only the mints that have nothing settled yet.
            let unsettled = unsettled.remove(&mint_id).unwrap_or_default();
            balances.push(MintBalance {
                mint_id,
                settled,
                unsettled,
            });
        }
        balances.extend(
            unsettled
                .into_iter()
                .map(|(mint_id, unsettled)| MintBalance {
                    mint_id,
                    settled: cashu::Amount::ZERO,
                    unsettled,
                }),
        );
        Ok(balances)
    }
}

/// Retryable: the exchange is held, but its proofs are not issued yet.
fn exchange_in_progress() -> Error {
    Error::ServiceUnavailable(SUError::Unknown)
}

async fn try_online_htlc(
    preimage: &str,
    repo: &dyn OnlineRepository,
    clowder: &dyn ClowderClient,
    factory: &dyn MintClientFactory,
    now: TStamp,
) -> Result<cashu::Amount> {
    let mut gran_total = cashu::Amount::ZERO;
    // Online preimages are fixed-size 32-byte hex; non-hex preimages are not online unlocks.
    let Some(hash) = online_hash_lock(preimage) else {
        return Ok(gran_total);
    };
    let foreign_proofs = repo.search_htlc(&hash).await?;
    let foreign_proofs = to_mint_proofs_map(foreign_proofs);

    for (mint_id, mut f_proofs) in foreign_proofs {
        let mint_url = clowder.get_mint_url_from_pk(&mint_id).await?;
        let foreign_client = factory.make_client(mint_url, mint_id).await?;
        let mut f_fingerprints = Vec::with_capacity(f_proofs.len());
        for proof in &mut f_proofs {
            proof.add_preimage(preimage.to_string());
            f_fingerprints.push(proof.y()?);
        }
        f_proofs = clowder.sign_p2pk_proofs(&f_proofs).await?;
        let new_proofs =
            signed_swap_with_foreign(f_proofs, clowder, foreign_client.as_ref(), now).await?;
        let total = new_proofs.total_amount().unwrap_or_default();
        repo.store(mint_id, new_proofs).await?;
        repo.remove_htlcs(&f_fingerprints).await?;
        gran_total += total;
    }
    if gran_total > cashu::Amount::ZERO {
        // Backed now, so no longer the reclaim routine's to burn.
        repo.remove_issued_by_hash(&hash).await?;
    }
    Ok(gran_total)
}

async fn try_offline_htlc_swap(
    preimage: &str,
    repo: &dyn OfflineRepository,
    issued: &dyn OnlineRepository,
    clowder: &dyn ClowderClient,
) -> Result<cashu::Amount> {
    let hash = offline_hash_lock(preimage);
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
        witness: None,
        secret,
        p2pk_e: None,
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
    // Backed now, so no longer the reclaim routine's to burn.
    issued.remove_issued_by_hash(&hash).await?;
    Ok(amount)
}

#[cfg(test)]
mod tests {

    use super::*;
    use bcr_common::{
        core, core_tests, ecash,
        wire::{
            attestation::{self as wire_attestation, IssuanceAttestation},
            swap as wire_swap,
        },
    };
    use bcr_wdc_utils::{keys as keys_utils, signatures::test_utils as signature_tests};
    use bitcoin::hashes::Hash;
    use bitcoin::hex::prelude::*;
    use mockall::predicate::*;

    fn generate_htlc_proof_for_online_exchange(
        keyset: &ecash::MintKeySet,
        amount: cashu::Amount,
        locktime: TStamp,
        wpk: cashu::PublicKey,
        mint: cashu::PublicKey,
    ) -> (cashu::Proof, String) {
        let preimage: [u8; 32] = rand::random();
        let preimage = format!("{:x}", preimage.as_hex());
        let conditions = cashu::SpendingConditions::new_htlc(
            preimage.clone(),
            Some(cashu::Conditions {
                locktime: Some(locktime.unix_timestamp() as u64),
                pubkeys: Some(vec![mint]),
                refund_keys: Some(vec![wpk]),
                ..Default::default()
            }),
        )
        .unwrap();
        let premints = cashu::PreMintSecrets::with_conditions(
            keyset.id.into(),
            amount,
            &cashu::amount::SplitTarget::None,
            &conditions,
            &keys_utils::to_fee_and_amounts(&core::keys::to_keyset(&keyset, None)),
        )
        .unwrap();
        assert_eq!(premints.blinded_messages().len(), 1);
        let blind = premints.blinded_messages()[0].clone();
        let signature = bcr_common::core::signature::sign_ecash(&keyset, &blind).unwrap();
        let proof = bcr_common::core::signature::unblind_ecash_signature(
            &bcr_wdc_utils::keys::to_keyset(&keyset, None),
            premints.into_iter().next().unwrap(),
            signature,
        )
        .unwrap();
        (proof, preimage)
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
        let (mut foreign_info, mut foreign_keyset) = core_tests::generate_random_ecash_keyset();
        let expiration = time::OffsetDateTime::now_utc() + time::Duration::days(7);
        foreign_keyset.final_expiry = Some(expiration.unix_timestamp() as u64);
        foreign_info.final_expiry = Some(expiration.unix_timestamp() as u64);
        let inputs = vec![
            generate_htlc_proof_for_online_exchange(
                &foreign_keyset.clone(),
                cashu::Amount::from(512),
                time::OffsetDateTime::now_utc() + time::Duration::minutes(90),
                cashu::PublicKey::from(wallet_kp.public_key()),
                cashu::PublicKey::from(myself_kp.public_key()),
            )
            .0,
            generate_htlc_proof_for_online_exchange(
                &foreign_keyset.clone(),
                cashu::Amount::from(256),
                time::OffsetDateTime::now_utc() + time::Duration::minutes(90),
                cashu::PublicKey::from(wallet_kp.public_key()),
                cashu::PublicKey::from(myself_kp.public_key()),
            )
            .0,
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
            .returning(move || Ok(myself_pk));
        let cloned_url = foreign_url.clone();
        clowder
            .expect_get_mint_url_from_pk()
            .with(eq(foreign_pk))
            .times(1)
            .returning(move |_| Ok(cloned_url.clone()));
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
                Ok(Box::new(foreign_client))
            });
        clowder
            .expect_check_htlc_proofs()
            .with(eq(foreign_pk), eq(inputs.clone()))
            .times(1)
            .returning(|_, _| Ok(()));
        let foreign_kid = foreign_keyset.id;
        let foreign_info = ecash::KeySetInfo::from(foreign_info);
        clowder
            .expect_get_keyset_info()
            .with(eq(foreign_pk), eq(cashu::Id::from(foreign_kid)))
            .times(1)
            .returning(move |_, _| Ok(foreign_info.clone()));
        let cloned_inputs = inputs.clone();
        clowder
            .expect_signal_online_exchange_event()
            .times(1)
            .with(eq(inputs.clone()), always(), eq(exchange_path.clone()))
            .returning(move |_, _, _| Ok(cloned_inputs.clone()));
        let (_, mut myself_keyset) = core_tests::generate_random_ecash_keyset();
        myself_keyset.final_expiry = Some(expiration.unix_timestamp() as u64);
        let cloned_keyset = bcr_wdc_utils::keys::to_keyset(&myself_keyset.clone(), None);
        onlinerepo
            .expect_store_htlc()
            .times(1)
            .returning(|_, _, _| Ok(()));
        onlinerepo
            .expect_store_issued()
            .times(1)
            .returning(|_, _, _| Ok(()));
        keys.expect_get_keyset_with_expiration()
            .with(eq(expiration.date()))
            .times(1)
            .returning(move |_| Ok(cloned_keyset.clone()));
        let cloned_keyset: cashu::MintKeySet = myself_keyset.clone().into();
        keys.expect_sign().times(1).returning(move |blinds| {
            let mut signatures = Vec::with_capacity(blinds.len());
            for blind in blinds {
                signatures.push(
                    bcr_common::core::signature::sign_ecash(&cloned_keyset.clone().into(), blind)
                        .unwrap(),
                );
            }
            Ok(signatures)
        });

        let srvc = Service {
            online_repo: Arc::new(onlinerepo),
            offline_repo: Arc::new(offlinerepo),
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
            mint_factory: Arc::new(factory),
            exchange_lock_margin_secs: 15 * 60,
            offline_exchange_lock_secs: 7 * 24 * 3600,
        };
        let proofs = srvc
            .online_exchange(inputs, exchange_path, time::OffsetDateTime::now_utc())
            .await
            .unwrap();
        assert_eq!(2, proofs.len());
    }

    #[tokio::test]
    async fn offline_exchange_works() {
        let mut onlinerepo = crate::foreign::MockOnlineRepository::new();
        let mut offlinerepo = crate::foreign::MockOfflineRepository::new();
        let mut keys = crate::foreign::MockKeysClient::new();
        let mut clowder = crate::foreign::MockClowderClient::new();
        let factory = crate::foreign::MockMintClientFactory::new();
        let foreign_kp = core::generate_random_keypair();
        let myself_kp = core::generate_random_keypair();
        let wallet_kp = core::generate_random_keypair();
        let foreign_url = reqwest::Url::parse("https://foreign-mint.example").unwrap();
        let (mut foreign_info, foreign_keyset) = core_tests::generate_random_ecash_keyset();
        let expiration = time::OffsetDateTime::now_utc() + time::Duration::days(7);
        foreign_info.final_expiry = Some(expiration.unix_timestamp() as u64);
        let originals = [
            generate_htlc_proof_for_online_exchange(
                &foreign_keyset.clone(),
                cashu::Amount::from(512),
                time::OffsetDateTime::now_utc() + time::Duration::minutes(90),
                cashu::PublicKey::from(wallet_kp.public_key()),
                cashu::PublicKey::from(myself_kp.public_key()),
            )
            .0,
            generate_htlc_proof_for_online_exchange(
                &foreign_keyset.clone(),
                cashu::Amount::from(256),
                time::OffsetDateTime::now_utc() + time::Duration::minutes(90),
                cashu::PublicKey::from(wallet_kp.public_key()),
                cashu::PublicKey::from(myself_kp.public_key()),
            )
            .0,
        ];
        let inputs = originals
            .iter()
            .map(|p| wire_keys::ProofFingerprint::try_from(p.clone()).unwrap())
            .collect::<Vec<_>>();
        let hashes = originals
            .iter()
            .map(|p| Sha256Hash::hash(p.secret.as_bytes()))
            .collect::<Vec<_>>();
        let wallet_pk = cashu::PublicKey::from(wallet_kp.public_key());
        let cloned_url = foreign_url.clone();
        let foreign_pk = foreign_kp.public_key();
        let digest = exchange_digest(&foreign_pk, &[1u8; 32], &inputs, &hashes, &wallet_pk);
        offlinerepo
            .expect_search_reservation()
            .times(1)
            .returning(|_| Ok(None));
        clowder
            .expect_can_accept_offline_exchange()
            .times(1)
            .with(eq(inputs.clone()))
            .returning(move |_| Ok((cloned_url.clone(), foreign_pk)));
        let foreign_kid = foreign_keyset.id;
        let foreign_info = ecash::KeySetInfo::from(foreign_info);
        clowder
            .expect_get_keyset_info()
            .with(eq(foreign_pk), eq(cashu::Id::from(foreign_kid)))
            .times(1)
            .returning(move |_, _| Ok(foreign_info.clone()));
        let (_, mut myself_keyset) = core_tests::generate_random_ecash_keyset();
        myself_keyset.final_expiry = Some(expiration.unix_timestamp() as u64);
        let cloned_keyset = bcr_wdc_utils::keys::to_keyset(&myself_keyset.clone(), None);
        keys.expect_get_keyset_with_expiration()
            .with(eq(expiration.date()))
            .times(1)
            .returning(move |_| Ok(cloned_keyset.clone()));
        // The exchange only proceeds once the node has recorded it.
        clowder
            .expect_record_offline_exchange()
            .times(1)
            .returning(move |_| {
                Ok(bcr_common::wire::exchange::RecordOfflineExchangeResponse {
                    evidence_digest: [1u8; 32],
                    exchange_digest: digest,
                })
            });
        let myself_pk = myself_kp.public_key();
        clowder
            .expect_get_myself_pk()
            .times(1)
            .returning(move || Ok(myself_pk));
        let now = time::OffsetDateTime::now_utc();
        let expires_at = now + time::Duration::seconds(7 * 24 * 3600);
        let mut seq = mockall::Sequence::new();
        offlinerepo
            .expect_reserve_exchange()
            .withf(move |r, ys| {
                r.exchange_digest == digest
                    && r.alpha_id == foreign_pk
                    && r.state == ReservationState::Reserved
                    && ys.len() == 2
            })
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _| Ok(true));
        let cloned_keyset: cashu::MintKeySet = myself_keyset.clone().into();
        keys.expect_sign()
            .times(1)
            .in_sequence(&mut seq)
            .returning(move |blinds| {
                let mut signatures = Vec::with_capacity(blinds.len());
                for blind in blinds {
                    signatures.push(
                        bcr_common::core::signature::sign_ecash(
                            &cloned_keyset.clone().into(),
                            blind,
                        )
                        .unwrap(),
                    );
                }
                Ok(signatures)
            });
        offlinerepo
            .expect_issue_reservation()
            .with(
                eq(digest),
                eq(foreign_pk),
                eq(inputs.clone()),
                eq(hashes.clone()),
                always(),
                eq(expires_at),
            )
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _, _, _, _, _| Ok(true));
        clowder
            .expect_signal_offline_exchange_event()
            .times(1)
            .with(
                eq(inputs.clone()),
                eq(hashes.clone()),
                eq(wallet_pk),
                always(),
                eq(Some(digest)),
                always(),
            )
            .in_sequence(&mut seq)
            .returning(|_, _, _, _, _, _| Ok(()));
        onlinerepo
            .expect_store_issued()
            .with(always(), eq(expires_at), always())
            .times(2)
            .in_sequence(&mut seq)
            .returning(|_, _, _| Ok(()));
        offlinerepo
            .expect_complete_reservation()
            .with(eq(digest))
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_| Ok(()));

        let srvc = Service {
            online_repo: Arc::new(onlinerepo),
            offline_repo: Arc::new(offlinerepo),
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
            mint_factory: Arc::new(factory),
            exchange_lock_margin_secs: 15 * 60,
            offline_exchange_lock_secs: 7 * 24 * 3600,
        };
        let proofs = srvc
            .offline_exchange(
                inputs,
                hashes.clone(),
                wallet_pk,
                secp256k1::global::SECP256K1.sign_schnorr(&exchange_message(&digest), &wallet_kp),
                now,
            )
            .await
            .unwrap();
        assert_eq!(2, proofs.len());
        // Issued offline eCash carries the shape the wallet and the reclaim routine rely on.
        for proof in &proofs {
            let (hash, conditions) = bcr_common::core::htlc::htlc_lock(proof).unwrap();
            assert!(hashes.contains(&hash));
            assert_eq!(
                conditions.locktime,
                Some(expires_at.unix_timestamp() as u64)
            );
            assert_eq!(conditions.pubkeys, Some(vec![wallet_pk]));
            assert_eq!(
                conditions.refund_keys,
                Some(vec![cashu::PublicKey::from(myself_pk)])
            );
            assert!(core::signature::is_offline_exchange_htlc(proof));
        }
    }

    struct RetryFixture {
        request: OfflineExchangeRequest,
        other_request: OfflineExchangeRequest,
        alpha_id: secp256k1::PublicKey,
        evidence_digest: [u8; 32],
        exchange_digest: [u8; 32],
        proof: cashu::Proof,
    }

    fn retry_fixture() -> RetryFixture {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/offline_exchange_retry.json"
        ))
        .unwrap();
        let field = |key: &str| fixture[key].clone();
        let digest = |key: &str| <[u8; 32]>::from_hex(fixture[key].as_str().unwrap()).unwrap();
        RetryFixture {
            request: serde_json::from_value(field("request")).unwrap(),
            other_request: serde_json::from_value(field("other_request")).unwrap(),
            alpha_id: serde_json::from_value(field("alpha_id")).unwrap(),
            evidence_digest: digest("evidence_digest"),
            exchange_digest: digest("exchange_digest"),
            proof: serde_json::from_value(field("proof")).unwrap(),
        }
    }

    /// Clowder and keys that accept, record and price the fixture's exchange once; the
    /// returned keyset signs it.
    fn fixture_exchange_mocks(
        f: &RetryFixture,
    ) -> (
        crate::foreign::MockClowderClient,
        crate::foreign::MockKeysClient,
        ecash::MintKeySet,
    ) {
        let mut clowder = crate::foreign::MockClowderClient::new();
        let mut keys = crate::foreign::MockKeysClient::new();
        let alpha_id = f.alpha_id;
        clowder
            .expect_can_accept_offline_exchange()
            .times(1)
            .returning(move |_| {
                Ok((
                    reqwest::Url::parse("https://alpha.example").unwrap(),
                    alpha_id,
                ))
            });
        let myself_pk = core::generate_random_keypair().public_key();
        clowder
            .expect_get_myself_pk()
            .times(1)
            .returning(move || Ok(myself_pk));
        let recorded = bcr_common::wire::exchange::RecordOfflineExchangeResponse {
            evidence_digest: f.evidence_digest,
            exchange_digest: f.exchange_digest,
        };
        clowder
            .expect_record_offline_exchange()
            .times(1)
            .returning(move |_| Ok(recorded.clone()));
        let expiration = time::OffsetDateTime::now_utc() + time::Duration::days(7);
        let (mut info, keyset) = core_tests::generate_random_ecash_keyset();
        info.final_expiry = Some(expiration.unix_timestamp() as u64);
        let info = ecash::KeySetInfo::from(info);
        clowder
            .expect_get_keyset_info()
            .times(1)
            .returning(move |_, _| Ok(info.clone()));
        let cloned_keyset = bcr_wdc_utils::keys::to_keyset(&keyset, None);
        keys.expect_get_keyset_with_expiration()
            .times(1)
            .returning(move |_| Ok(cloned_keyset.clone()));
        (clowder, keys, keyset)
    }

    fn sign_all(
        keyset: &ecash::MintKeySet,
        blinds: &[cashu::BlindedMessage],
    ) -> Result<Vec<cashu::BlindSignature>> {
        Ok(blinds
            .iter()
            .map(|blind| bcr_common::core::signature::sign_ecash(keyset, blind).unwrap())
            .collect())
    }

    fn fixture_service(
        offline_repo: Arc<dyn OfflineRepository>,
        online_repo: crate::foreign::MockOnlineRepository,
        keys: crate::foreign::MockKeysClient,
        clowder: crate::foreign::MockClowderClient,
    ) -> Service {
        Service {
            online_repo: Arc::new(online_repo),
            offline_repo,
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
            mint_factory: Arc::new(crate::foreign::MockMintClientFactory::new()),
            exchange_lock_margin_secs: 15 * 60,
            offline_exchange_lock_secs: 7 * 24 * 3600,
        }
    }

    async fn exchange(
        srvc: &Service,
        request: &OfflineExchangeRequest,
        now: TStamp,
    ) -> Result<Vec<cashu::Proof>> {
        srvc.offline_exchange(
            request.fingerprints.clone(),
            request.hashes.clone(),
            request.wallet_pk,
            request.wallet_signature,
            now,
        )
        .await
    }

    fn proofs_json(proofs: &[cashu::Proof]) -> String {
        serde_json::to_string(proofs).unwrap()
    }

    /// The fixture's exchange, already signed, stored and marked complete.
    async fn completed_fixture_store(
        f: &RetryFixture,
    ) -> Arc<crate::persistence::inmemory::OfflineRepository> {
        let repo = Arc::new(crate::persistence::inmemory::OfflineRepository::default());
        let ys: Vec<_> = f.request.fingerprints.iter().map(|fp| fp.y).collect();
        let reservation = OfflineReservation {
            exchange_digest: f.exchange_digest,
            alpha_id: f.alpha_id,
            evidence_digest: f.evidence_digest,
            state: ReservationState::Reserved,
        };
        assert!(repo.reserve_exchange(reservation, &ys).await.unwrap());
        assert!(repo
            .issue_reservation(
                f.exchange_digest,
                f.alpha_id,
                f.request.fingerprints.clone(),
                f.request.hashes.clone(),
                vec![f.proof.clone()],
                time::OffsetDateTime::now_utc(),
            )
            .await
            .unwrap());
        repo.complete_reservation(f.exchange_digest).await.unwrap();
        repo
    }

    #[tokio::test]
    async fn offline_exchange_replay_returns_stored_proofs() {
        let f = retry_fixture();
        let (mut clowder, mut keys, keyset) = fixture_exchange_mocks(&f);
        keys.expect_sign()
            .times(1)
            .returning(move |blinds| sign_all(&keyset, blinds));
        clowder
            .expect_signal_offline_exchange_event()
            .times(1)
            .returning(|_, _, _, _, _, _| Ok(()));
        let mut online = crate::foreign::MockOnlineRepository::new();
        online
            .expect_store_issued()
            .times(1)
            .returning(|_, _, _| Ok(()));
        let repo = Arc::new(crate::persistence::inmemory::OfflineRepository::default());
        let srvc = fixture_service(repo, online, keys, clowder);
        let now = time::OffsetDateTime::now_utc();

        let first = exchange(&srvc, &f.request, now).await.unwrap();
        let second = exchange(&srvc, &f.request, now + time::Duration::hours(1))
            .await
            .unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(proofs_json(&first), proofs_json(&second));
    }

    #[tokio::test]
    async fn offline_exchange_other_request_is_refused() {
        let f = retry_fixture();
        let (mut clowder, mut keys, keyset) = fixture_exchange_mocks(&f);
        keys.expect_sign()
            .times(1)
            .returning(move |blinds| sign_all(&keyset, blinds));
        clowder
            .expect_signal_offline_exchange_event()
            .times(1)
            .returning(|_, _, _, _, _, _| Ok(()));
        let mut online = crate::foreign::MockOnlineRepository::new();
        online
            .expect_store_issued()
            .times(1)
            .returning(|_, _, _| Ok(()));
        let repo = Arc::new(crate::persistence::inmemory::OfflineRepository::default());
        let srvc = fixture_service(repo, online, keys, clowder);
        let now = time::OffsetDateTime::now_utc();

        exchange(&srvc, &f.request, now).await.unwrap();
        let other = exchange(&srvc, &f.other_request, now).await;
        assert!(matches!(other, Err(Error::InvalidInput(_))));
    }

    #[tokio::test]
    async fn offline_exchange_failed_sign_replay_is_retryable() {
        let f = retry_fixture();
        let (mut clowder, mut keys, _) = fixture_exchange_mocks(&f);
        keys.expect_sign()
            .times(1)
            .returning(|_| Err(Error::InvalidInput(String::from("keys unavailable"))));
        clowder.expect_signal_offline_exchange_event().never();
        let mut online = crate::foreign::MockOnlineRepository::new();
        online.expect_store_issued().never();
        let repo = Arc::new(crate::persistence::inmemory::OfflineRepository::default());
        let srvc = fixture_service(repo, online, keys, clowder);
        let now = time::OffsetDateTime::now_utc();

        assert!(exchange(&srvc, &f.request, now).await.is_err());
        let replay = exchange(&srvc, &f.request, now).await;
        assert!(matches!(replay, Err(Error::ServiceUnavailable(_))));
    }

    #[tokio::test]
    async fn offline_exchange_replay_completed_has_no_side_effects() {
        let f = retry_fixture();
        let repo = completed_fixture_store(&f).await;
        let mut clowder = crate::foreign::MockClowderClient::new();
        clowder.expect_can_accept_offline_exchange().never();
        clowder.expect_record_offline_exchange().never();
        clowder.expect_signal_offline_exchange_event().never();
        let mut keys = crate::foreign::MockKeysClient::new();
        keys.expect_sign().never();
        let mut online = crate::foreign::MockOnlineRepository::new();
        online.expect_store_issued().never();
        let srvc = fixture_service(repo, online, keys, clowder);

        let proofs = exchange(&srvc, &f.request, time::OffsetDateTime::now_utc())
            .await
            .unwrap();
        assert_eq!(proofs_json(&proofs), proofs_json(&[f.proof]));
    }

    /// The fixture's exchange stopped after its proofs were stored, before they were
    /// announced; the proofs are HTLC-locked like issued offline eCash.
    async fn issued_fixture_store(
        f: &RetryFixture,
        expires_at: TStamp,
    ) -> (
        Arc<crate::persistence::inmemory::OfflineRepository>,
        Vec<cashu::Proof>,
    ) {
        let repo = Arc::new(crate::persistence::inmemory::OfflineRepository::default());
        let ys: Vec<_> = f.request.fingerprints.iter().map(|fp| fp.y).collect();
        let reservation = OfflineReservation {
            exchange_digest: f.exchange_digest,
            alpha_id: f.alpha_id,
            evidence_digest: f.evidence_digest,
            state: ReservationState::Reserved,
        };
        assert!(repo.reserve_exchange(reservation, &ys).await.unwrap());
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = vec![
            generate_htlc_proof_for_online_exchange(
                &keyset,
                cashu::Amount::from(8),
                expires_at,
                f.request.wallet_pk,
                cashu::PublicKey::from(core::generate_random_keypair().public_key()),
            )
            .0,
        ];
        assert!(repo
            .issue_reservation(
                f.exchange_digest,
                f.alpha_id,
                f.request.fingerprints.clone(),
                f.request.hashes.clone(),
                proofs.clone(),
                expires_at,
            )
            .await
            .unwrap());
        (repo, proofs)
    }

    #[tokio::test]
    async fn offline_exchange_replay_resumes_tail() {
        let f = retry_fixture();
        let expires_at = time::OffsetDateTime::now_utc() + time::Duration::days(7);
        let (repo, stored) = issued_fixture_store(&f, expires_at).await;
        let mut clowder = crate::foreign::MockClowderClient::new();
        clowder.expect_record_offline_exchange().never();
        let cloned = stored.clone();
        clowder
            .expect_signal_offline_exchange_event()
            .withf(move |_, _, _, proofs, _, _| *proofs == cloned)
            .times(1)
            .returning(|_, _, _, _, _, _| Ok(()));
        let mut keys = crate::foreign::MockKeysClient::new();
        keys.expect_sign().never();
        let mut online = crate::foreign::MockOnlineRepository::new();
        online
            .expect_store_issued()
            .with(always(), eq(expires_at), always())
            .times(1)
            .returning(|_, _, _| Ok(()));
        let srvc = fixture_service(repo.clone(), online, keys, clowder);
        let ys: Vec<_> = f.request.fingerprints.iter().map(|fp| fp.y).collect();

        let proofs = exchange(&srvc, &f.request, time::OffsetDateTime::now_utc())
            .await
            .unwrap();
        assert_eq!(proofs_json(&proofs), proofs_json(&stored));
        let completed = repo.search_reservation(&ys).await.unwrap().unwrap();
        assert_eq!(completed.state, ReservationState::Complete(stored));
    }

    /// Replays the Issued fixture exchange with a signal failing with `signal_err`, and
    /// checks it stays Issued with its stored proofs and expiry.
    async fn replay_with_failing_tail(signal_err: fn() -> Error) -> Result<Vec<cashu::Proof>> {
        let f = retry_fixture();
        let expires_at = time::OffsetDateTime::now_utc() + time::Duration::days(7);
        let (repo, stored) = issued_fixture_store(&f, expires_at).await;
        let mut clowder = crate::foreign::MockClowderClient::new();
        clowder
            .expect_signal_offline_exchange_event()
            .times(1)
            .returning(move |_, _, _, _, _, _| Err(signal_err()));
        let mut keys = crate::foreign::MockKeysClient::new();
        keys.expect_sign().never();
        let mut online = crate::foreign::MockOnlineRepository::new();
        online.expect_store_issued().never();
        let srvc = fixture_service(repo.clone(), online, keys, clowder);
        let ys: Vec<_> = f.request.fingerprints.iter().map(|fp| fp.y).collect();

        let replay = exchange(&srvc, &f.request, time::OffsetDateTime::now_utc()).await;
        let still = repo.search_reservation(&ys).await.unwrap().unwrap();
        assert_eq!(
            still.state,
            ReservationState::Issued {
                proofs: stored,
                expires_at
            }
        );
        replay
    }

    fn rejected(r: bcr_common::wire::clowder::ClowderRejection) -> Error {
        Error::ClowderNatsClient(ClowderClientError::Rejected(r))
    }

    #[tokio::test]
    async fn offline_exchange_replay_tail_fails_is_retryable() {
        let replay =
            replay_with_failing_tail(|| Error::InvalidInput(String::from("clowder unavailable")))
                .await;
        assert!(matches!(replay, Err(Error::ServiceUnavailable(_))));
    }

    #[tokio::test]
    async fn offline_exchange_replay_tail_transient_rejection_is_retryable() {
        let replay = replay_with_failing_tail(|| {
            rejected(bcr_common::wire::clowder::ClowderRejection::LedgerBusy)
        })
        .await;
        assert!(matches!(replay, Err(Error::ServiceUnavailable(_))));
    }

    #[tokio::test]
    async fn offline_exchange_replay_tail_refused_is_not_retryable() {
        let replay = replay_with_failing_tail(|| {
            rejected(bcr_common::wire::clowder::ClowderRejection::InvalidProof)
        })
        .await;
        assert!(matches!(
            replay,
            Err(Error::ClowderNatsClient(ClowderClientError::Rejected(_)))
        ));
    }

    #[tokio::test]
    async fn offline_exchange_replay_bad_signature_is_refused() {
        let f = retry_fixture();
        let repo = completed_fixture_store(&f).await;
        let mut keys = crate::foreign::MockKeysClient::new();
        keys.expect_sign().never();
        let srvc = fixture_service(
            repo,
            crate::foreign::MockOnlineRepository::new(),
            keys,
            crate::foreign::MockClowderClient::new(),
        );
        let forged = OfflineExchangeRequest {
            wallet_signature: f.other_request.wallet_signature,
            ..f.request.clone()
        };

        let replay = exchange(&srvc, &forged, time::OffsetDateTime::now_utc()).await;
        assert!(matches!(replay, Err(Error::InvalidInput(_))));
    }

    fn redeem_request(amounts: &[u64]) -> bcr_common::wire::exchange::RedeemOfflineExchangeRequest {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts: Vec<cashu::Amount> = amounts.iter().map(|a| cashu::Amount::from(*a)).collect();
        let outputs = core_tests::generate_random_ecash_blindedmessages(keyset.id.into(), &amounts)
            .into_iter()
            .map(|(msg, _, _)| msg)
            .collect();
        bcr_common::wire::exchange::RedeemOfflineExchangeRequest::new(
            &core::generate_random_keypair().public_key(),
            [7u8; 32],
            outputs,
            &core::generate_random_keypair(),
        )
    }

    // A mint with only settled, one with only unsettled, and one with both.
    #[tokio::test]
    async fn balance_unions_the_two_stores() {
        let settled_only = core::generate_random_keypair().public_key();
        let unsettled_only = core::generate_random_keypair().public_key();
        let both = core::generate_random_keypair().public_key();

        let mut online_repo = crate::foreign::MockOnlineRepository::new();
        online_repo
            .expect_settled_balance()
            .times(1)
            .returning(move || {
                Ok(HashMap::from([
                    (settled_only, cashu::Amount::from(8u64)),
                    (both, cashu::Amount::from(16u64)),
                ]))
            });
        let mut offline_repo = crate::foreign::MockOfflineRepository::new();
        offline_repo
            .expect_unsettled_balance()
            .times(1)
            .returning(move || {
                Ok(HashMap::from([
                    (unsettled_only, cashu::Amount::from(2u64)),
                    (both, cashu::Amount::from(4u64)),
                ]))
            });

        let service = Service {
            online_repo: Arc::new(online_repo),
            offline_repo: Arc::new(offline_repo),
            keys: Arc::new(crate::foreign::MockKeysClient::new()),
            clowder: Arc::new(crate::foreign::MockClowderClient::new()),
            mint_factory: Arc::new(crate::foreign::MockMintClientFactory::new()),
            exchange_lock_margin_secs: 15 * 60,
            offline_exchange_lock_secs: 7 * 24 * 3600,
        };
        let balances = service.balance().await.unwrap();

        assert_eq!(balances.len(), 3);
        let by_mint: HashMap<_, _> = balances
            .into_iter()
            .map(|b| (b.mint_id, (b.settled, b.unsettled)))
            .collect();
        assert_eq!(
            by_mint[&settled_only],
            (cashu::Amount::from(8u64), cashu::Amount::ZERO)
        );
        assert_eq!(
            by_mint[&unsettled_only],
            (cashu::Amount::ZERO, cashu::Amount::from(2u64))
        );
        assert_eq!(
            by_mint[&both],
            (cashu::Amount::from(16u64), cashu::Amount::from(4u64))
        );
    }

    fn redeem_service(
        offline_repo: crate::foreign::MockOfflineRepository,
        keys: crate::foreign::MockKeysClient,
        clowder: crate::foreign::MockClowderClient,
    ) -> Service {
        Service {
            online_repo: Arc::new(crate::foreign::MockOnlineRepository::new()),
            offline_repo: Arc::new(offline_repo),
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
            mint_factory: Arc::new(crate::foreign::MockMintClientFactory::new()),
            exchange_lock_margin_secs: 15 * 60,
            offline_exchange_lock_secs: 7 * 24 * 3600,
        }
    }

    // Refused before anything is signed, or restore hands over the signatures anyway.
    #[tokio::test]
    async fn redeem_over_the_authorised_amount_never_signs() {
        let request = redeem_request(&[1, 2]);
        let mut clowder = crate::foreign::MockClowderClient::new();
        clowder
            .expect_redeem_offline_exchange()
            .times(1)
            .returning(|_| {
                Ok(
                    bcr_common::wire::clowder::RedeemOfflineExchangeAuthorization {
                        amount: cashu::Amount::from(2u64),
                    },
                )
            });
        let mut keys = crate::foreign::MockKeysClient::new();
        keys.expect_sign().never();
        let mut offline_repo = crate::foreign::MockOfflineRepository::new();
        offline_repo.expect_claim_redemption().never();

        let srvc = redeem_service(offline_repo, keys, clowder);
        assert!(srvc.redeem_offline_exchange(request).await.is_err());
    }

    // A replay with fresh outputs signs nothing.
    #[tokio::test]
    async fn redeem_replay_never_signs() {
        let request = redeem_request(&[1, 2]);
        let mut clowder = crate::foreign::MockClowderClient::new();
        clowder
            .expect_redeem_offline_exchange()
            .times(1)
            .returning(|_| {
                Ok(
                    bcr_common::wire::clowder::RedeemOfflineExchangeAuthorization {
                        amount: cashu::Amount::from(3u64),
                    },
                )
            });
        let mut offline_repo = crate::foreign::MockOfflineRepository::new();
        offline_repo
            .expect_claim_redemption()
            .with(eq([7u8; 32]))
            .times(1)
            .returning(|_| Ok(false));
        let mut keys = crate::foreign::MockKeysClient::new();
        keys.expect_sign().never();

        let srvc = redeem_service(offline_repo, keys, clowder);
        assert!(srvc.redeem_offline_exchange(request).await.is_err());
    }

    // Claim the entry, then sign exactly what it owes.
    #[tokio::test]
    async fn redeem_signs_what_the_entry_owes() {
        let request = redeem_request(&[1, 2]);
        let mut clowder = crate::foreign::MockClowderClient::new();
        clowder
            .expect_redeem_offline_exchange()
            .times(1)
            .returning(|_| {
                Ok(
                    bcr_common::wire::clowder::RedeemOfflineExchangeAuthorization {
                        amount: cashu::Amount::from(3u64),
                    },
                )
            });
        clowder
            .expect_signal_offline_redeem_event()
            .times(1)
            .returning(|_, _| Ok(()));
        let mut offline_repo = crate::foreign::MockOfflineRepository::new();
        offline_repo
            .expect_claim_redemption()
            .times(1)
            .returning(|_| Ok(true));
        let mut keys = crate::foreign::MockKeysClient::new();
        keys.expect_sign().times(1).returning(|blinds| {
            let (_, keyset) = core_tests::generate_random_ecash_keyset();
            Ok(blinds
                .iter()
                .map(|b| bcr_common::core::signature::sign_ecash(&keyset.clone(), b).unwrap())
                .collect())
        });

        let srvc = redeem_service(offline_repo, keys, clowder);
        let signatures = srvc.redeem_offline_exchange(request).await.unwrap();
        assert_eq!(2, signatures.len());
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
        let (foreign_proof, preimage) = generate_htlc_proof_for_online_exchange(
            &foreign_keyset.clone(),
            cashu::Amount::from(256),
            time::OffsetDateTime::now_utc() + time::Duration::minutes(90),
            cashu::PublicKey::from(wallet_kp.public_key()),
            cashu::PublicKey::from(myself_kp.public_key()),
        );
        let preimage_bytes = <[u8; 32]>::from_hex(&preimage).unwrap();
        let hash = Sha256Hash::hash(&preimage_bytes);
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
        ///// expectations for signed swap with foreign
        let mut foreign_client_mock = crate::foreign::MockForeignClient::new();
        foreign_client_mock
            .expect_get_foreign_pk()
            .times(1)
            .returning(move || foreign_kp.public_key());
        clowder
            .expect_get_keyset_info()
            .with(
                eq(foreign_kp.public_key()),
                eq(cashu::Id::from(foreign_keyset.id)),
            )
            .times(1)
            .returning(move |_, _| Ok(ecash::KeySetInfo::from(foreign_kinfo.clone())));
        let cloned_keyset = keys_utils::to_keyset(&foreign_keyset, None);
        clowder
            .expect_get_keyset()
            .with(
                eq(foreign_kp.public_key()),
                eq(cashu::Id::from(foreign_keyset.id)),
            )
            .times(1)
            .returning(move |_, _| Ok(cloned_keyset.clone()));
        foreign_client_mock
            .expect_prepare_swap_commitment_request()
            .times(1)
            .returning(move |inp, outp, now| {
                let fps = inp
                    .iter()
                    .map(|p| wire_keys::ProofFingerprint::try_from(p.clone()).unwrap())
                    .collect::<Vec<_>>();
                let attested = wire_attestation::AttestedFingerprints {
                    inputs: fps.clone(),
                    attestation: IssuanceAttestation {
                        beta_id: core::generate_random_keypair().public_key(),
                        fp_digest: Default::default(),
                        coords_mac: Default::default(),
                        signature: signature_tests::random_schnorr_signature(),
                    },
                };
                Ok(wire_swap::SwapCommitmentRequest {
                    inputs: attested,
                    outputs: outp.into_iter().map(From::from).collect(),
                    expiry: now.unix_timestamp() as u64,
                    wallet_key: core::generate_random_keypair().public_key(),
                })
            });
        clowder
            .expect_sign_swap_commitment_request()
            .times(1)
            .returning(|_| Ok((String::new(), signature_tests::random_schnorr_signature())));
        foreign_client_mock
            .expect_commit_swap_with_signature()
            .times(1)
            .returning(|_, _| Ok((String::new(), signature_tests::random_schnorr_signature())));
        foreign_client_mock
            .expect_swap()
            .times(1)
            .returning(move |inputs, outputs, _| {
                let mut signatures = Vec::with_capacity(inputs.len());
                for blind in outputs {
                    let signature =
                        bcr_common::core::signature::sign_ecash(&foreign_keyset.clone(), &blind)
                            .unwrap();
                    signatures.push(signature);
                }
                Ok(signatures)
            });

        factory
            .expect_make_client()
            .with(eq(foreign_url.clone()), always())
            .times(1)
            .return_once(move |_, _| Ok(Box::new(foreign_client_mock)));
        onlinerepo.expect_store().times(1).returning(|_, _| Ok(()));
        let foreign_y = foreign_proof.y().unwrap();
        onlinerepo
            .expect_remove_htlcs()
            .with(eq(vec![foreign_y]))
            .times(1)
            .returning(|_| Ok(()));
        onlinerepo
            .expect_remove_issued_by_hash()
            .with(eq(hash))
            .times(1)
            .returning(|_| Ok(()));
        let srvc = Service {
            online_repo: Arc::new(onlinerepo),
            offline_repo: Arc::new(offlinerepo),
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
            mint_factory: Arc::new(factory),
            exchange_lock_margin_secs: 15 * 60,
            offline_exchange_lock_secs: 7 * 24 * 3600,
        };
        let amount = srvc
            .try_swap_htlc(&preimage, time::OffsetDateTime::now_utc())
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
        let (foreign_proof, _) = generate_htlc_proof_for_online_exchange(
            &foreign_keyset.clone(),
            cashu::Amount::from(256),
            time::OffsetDateTime::now_utc() + time::Duration::minutes(90),
            cashu::PublicKey::from(wallet_kp.public_key()),
            cashu::PublicKey::from(myself_kp.public_key()),
        );
        // Offline-exchange preimage is the original alpha proof's secret string (not 32-byte hex),
        // so the online path decodes nothing and falls through to the offline swap.
        let preimage = foreign_proof.secret.to_string();
        let hash = Sha256Hash::hash(foreign_proof.secret.as_bytes());
        let search_response = (
            foreign_kp.public_key(),
            bcr_common::wire::keys::ProofFingerprint::try_from(foreign_proof.clone()).unwrap(),
        );
        offlinerepo
            .expect_search_fp()
            .with(eq(hash))
            .times(1)
            .returning(move |_| Ok(Some(search_response.clone())));
        let foreign_kid = foreign_keyset.id;
        let foreign_pk = foreign_kp.public_key();
        let cloned_keyset = keys_utils::to_keyset(&foreign_keyset, None);
        clowder
            .expect_get_keyset()
            .with(eq(foreign_pk), eq(cashu::Id::from(foreign_kid)))
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
        onlinerepo
            .expect_remove_issued_by_hash()
            .with(eq(hash))
            .times(1)
            .returning(|_| Ok(()));
        let srvc = Service {
            online_repo: Arc::new(onlinerepo),
            offline_repo: Arc::new(offlinerepo),
            keys: Arc::new(keys),
            clowder: Arc::new(clowder),
            mint_factory: Arc::new(factory),
            exchange_lock_margin_secs: 15 * 60,
            offline_exchange_lock_secs: 7 * 24 * 3600,
        };
        let amount = srvc
            .try_swap_htlc(&preimage, time::OffsetDateTime::now_utc())
            .await
            .unwrap();
        assert_eq!(cashu::Amount::from(256), amount);
    }

    #[test]
    fn fixture_exchange_digest() {
        let f = retry_fixture();
        let digest = exchange_digest(
            &f.alpha_id,
            &f.evidence_digest,
            &f.request.fingerprints,
            &f.request.hashes,
            &f.request.wallet_pk,
        );
        assert_eq!(digest, f.exchange_digest);
    }
}
