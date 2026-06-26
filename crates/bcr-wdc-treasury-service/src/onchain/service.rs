// ----- standard library imports
use std::{collections::HashSet, str::FromStr, sync::Arc};
// ----- extra library imports
use bcr_common::{
    cashu::{self, ProofsMethods},
    wire::{attestation as wire_attestation, melt as wire_melt, mint as wire_mint},
};
use bitcoin::secp256k1::PublicKey;
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    onchain::{
        self, ClowderClient, DeniedMeltOperation, MintOperation, MintStatus, Repository,
        WildcatClient,
    },
    TStamp,
};

// ----- end imports

pub struct Service {
    pub wdc: Arc<dyn WildcatClient>,
    pub repo: Arc<dyn Repository>,
    pub clowder_cl: Arc<dyn ClowderClient>,
    pub quote_expiry: chrono::Duration,
    pub min_mint_threshold: bitcoin::Amount,
    pub min_melt_threshold: bitcoin::Amount,
    pub alpha_id: PublicKey,
}

impl Service {
    pub async fn create_onchain_mint_quote(
        &self,
        request: wire_mint::OnchainMintQuoteRequest,
        now: TStamp,
    ) -> Result<wire_mint::OnchainMintQuoteResponse> {
        bcr_wdc_utils::signatures::basic_blinds_checks(&request.blinded_messages)
            .map_err(|e| Error::InvalidInput(e.to_string()))?;
        let qid = Uuid::new_v4();
        let blinds_camount = request
            .blinded_messages
            .iter()
            .fold(cashu::Amount::ZERO, |total, b| total + b.amount);
        let blinds_amount = bitcoin::Amount::from_sat(blinds_camount.into());
        if blinds_amount < self.min_mint_threshold {
            return Err(Error::InvalidInput(String::from("mint amount too low")));
        }
        let kid = self.wdc.get_active_keyset().await?;
        let same_kid = request.blinded_messages.iter().all(|b| b.keyset_id == kid);
        if !same_kid {
            return Err(Error::InvalidInput(String::from("invalid keyset id")));
        }
        let address = self
            .clowder_cl
            .request_onchain_mint_address(qid, kid)
            .await?;
        let expiry = now + self.quote_expiry;
        let mintop = MintOperation {
            qid,
            kid,
            target: blinds_amount,
            recipient: address.as_unchecked().clone(),
            expiry,
            status: MintStatus::Pending {
                blinds: request.blinded_messages.clone(),
            },
        };
        self.repo.store_mintop(mintop).await?;
        let body = wire_mint::OnchainMintQuoteResponseBody {
            quote: qid,
            address: address.to_string(),
            payment_amount: bitcoin::Amount::from_sat(blinds_camount.into()),
            blinded_messages: request.blinded_messages,
            expiry: expiry.timestamp().max(0) as u64,
            wallet_key: request.wallet_key,
        };

        let (content, commitment) = self.clowder_cl.sign_onchain_mint_response(&body).await?;
        let response = wire_mint::OnchainMintQuoteResponse {
            commitment,
            content,
        };
        Ok(response)
    }

    pub async fn create_onchain_melt_quote(
        &self,
        request: wire_melt::MeltQuoteOnchainRequest,
        now: TStamp,
    ) -> Result<wire_melt::MeltQuoteOnchainResponse> {
        let wire_melt::MeltQuoteOnchainRequest {
            inputs,
            wallet_key,
            address,
        } = request;
        // valid fingerprints
        if inputs.inputs.is_empty() {
            return Err(Error::InvalidInput(String::from("missing inputs")));
        }
        // authenticate the Beta issuance attestation bound to this commitment
        self.clowder_cl
            .authenticate_attestation(&self.alpha_id, &inputs)
            .await?;
        let core_fps: Vec<_> = inputs
            .inputs
            .iter()
            .cloned()
            .map(bcr_common::core::signature::ProofFingerprint::from)
            .collect();
        bcr_wdc_utils::signatures::basic_fingerprints_checks(&core_fps)?;
        self.wdc.verify_fingerprints(&inputs.inputs).await?;
        // Unspent fingerprints
        let mut input_ys = inputs.inputs.iter().map(|fp| fp.y).collect::<Vec<_>>();
        let states = self.wdc.check_spendable(input_ys.clone()).await?;
        let all_unspent = states
            .iter()
            .all(|s| matches!(s.state, cashu::State::Unspent));
        if !all_unspent {
            return Err(Error::InvalidInput(String::from("proofs already spent")));
        }
        // unlocked fingerprints
        let pending_ops = self.retrieve_pending_meltops(now).await?;
        cross_check_locked_fps(input_ys.clone(), &pending_ops)?;
        // amount > dust (after fees)
        let input_total =
            bitcoin::Amount::from_sat(inputs.inputs.iter().map(|fp| fp.amount).sum::<u64>());
        // valid address
        let checked_address = self
            .clowder_cl
            .verify_onchain_address(address.clone())
            .await?;
        // sufficient amount for fees
        let reserve = self.clowder_cl.get_onchain_reserve().await?;
        let admin_fees = self.calculate_melt_fees(reserve, input_total);
        let network_fees = self
            .clowder_cl
            .estimate_onchain_fees(input_total - admin_fees)
            .await?;
        if input_total < admin_fees + network_fees + self.min_melt_threshold {
            return Err(Error::InvalidInput(String::from(
                "insufficient funds to cover fees",
            )));
        }
        // insufficient funds in clowder
        let pending_amount = pending_ops
            .iter()
            .map(|op| op.target)
            .sum::<bitcoin::Amount>();
        if input_total > reserve - pending_amount {
            let op = DeniedMeltOperation {
                qid: Uuid::new_v4(),
                inputs: input_total,
                created: now,
            };
            self.repo.store_denied_meltop(op).await?;
            return Err(Error::Unavailable(String::from(
                "melt operation temporarily suspended, insufficient on-chain reserve, try again later")));
        }
        // all ok, proceed
        let target = input_total - admin_fees - network_fees;
        let expiry = now + self.quote_expiry;
        let qid = Uuid::new_v4();
        let body = wire_melt::MeltQuoteOnchainResponseBody {
            quote: qid,
            inputs: inputs.clone(),
            address: address.clone(),
            amount: target,
            expiry: expiry.timestamp().max(0) as u64,
            wallet_key,
        };
        let (content, commitment) = self
            .clowder_cl
            .sign_onchain_melt_response(&body, admin_fees, network_fees)
            .await?;
        input_ys.sort();
        let op = onchain::MeltOperation {
            qid,
            available: cashu::Amount::from(input_total.to_sat()),
            address: checked_address.to_string(),
            target,
            fees: cashu::Amount::from(admin_fees.to_sat()),
            expiry,
            wallet_key,
            input_ys,
            fp_digest: inputs.attestation.fp_digest,
            commitment,
            status: onchain::MeltStatus::Pending,
        };
        let inputs = op.input_ys.clone();
        let expiry = op.expiry;
        self.repo.store_meltop(op, now).await?;
        match self.wdc.reserve_inputs(inputs, expiry).await {
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("reserve_inputs failed with {e}");
                let rs = self
                    .repo
                    .update_meltop_status(qid, onchain::MeltStatus::Canceled)
                    .await;
                if let Err(e2) = rs {
                    tracing::error!(
                        "DB Failure, lost MeltStatus update for canceled MeltOp {qid}: {e2}"
                    );
                }
                return Err(e);
            }
        };
        Ok(wire_melt::MeltQuoteOnchainResponse {
            content,
            commitment,
        })
    }

    async fn retrieve_pending_meltops(&self, now: TStamp) -> Result<Vec<onchain::MeltOperation>> {
        let pendings_ids = self.repo.list_pending_meltops(now).await?;
        let mut ops: Vec<onchain::MeltOperation> = Vec::with_capacity(pendings_ids.len());
        for id in pendings_ids {
            let op = self.repo.load_meltop(id).await;
            let Ok(op) = op else {
                tracing::error!("DB Failure, lost access to pending MeltOp with id {id}");
                continue;
            };
            if op.expiry > now {
                ops.push(op);
            } else {
                let res = self
                    .repo
                    .update_meltop_status(id, onchain::MeltStatus::Expired)
                    .await;
                if let Err(e) = res {
                    tracing::error!(
                        "DB Failure, lost access to update expired MeltOp with id {id}: {e}"
                    );
                }
            }
        }
        Ok(ops)
    }

    fn calculate_melt_fees(
        &self,
        _reserve: bitcoin::Amount,
        amount: bitcoin::Amount,
    ) -> bitcoin::Amount {
        const MELT_FEES_MULTIPLIER: u64 = 100;
        let fees_sat = amount.to_sat().div_ceil(MELT_FEES_MULTIPLIER);
        bitcoin::Amount::from_sat(fees_sat)
    }

    pub async fn list_denied_meltops(&self) -> Result<Vec<DeniedMeltOperation>> {
        self.repo.list_denied_meltops().await
    }

    pub async fn delete_denied_meltop(&self, qid: Uuid) -> Result<()> {
        self.repo.delete_denied_meltop(qid).await
    }

    pub async fn melt_onchain(
        &self,
        request: wire_melt::MeltOnchainRequest,
        now: TStamp,
        vault: &dyn onchain::VaultService,
    ) -> Result<wire_melt::MeltOnchainResponse> {
        let wire_melt::MeltOnchainRequest { inputs, .. } = request;
        // verify proofs
        bcr_wdc_utils::signatures::basic_proofs_checks(&inputs)?;
        self.wdc.verify_proofs(&inputs).await?;
        // verify not spent
        let p_ys = inputs.ys()?;
        let states = self.wdc.check_spendable(p_ys.clone()).await?;
        let any_spent = states
            .iter()
            .any(|s| matches!(s.state, cashu::State::Spent));
        if any_spent {
            return Err(Error::InvalidInput(String::from("proofs already spent")));
        }
        // load melt operation
        let qid = request.quote;
        let op = self.repo.load_meltop(qid).await?;
        if now > op.expiry {
            return Err(Error::InvalidInput(String::from("Melt quote has expired")));
        }
        let input_fps = wire_attestation::project_to_fingerprints(&inputs)
            .map_err(|e| Error::InvalidInput(e.to_string()))?;
        if wire_attestation::fp_digest(&input_fps) != op.fp_digest {
            return Err(Error::InvalidInput(String::from(
                "melt proofs != committed fingerprints",
            )));
        }
        // attestation was authenticated at quote time; op.fp_digest binds these proofs to it
        let unchecked = bitcoin::Address::from_str(&op.address)?;
        let recipient = self.clowder_cl.verify_onchain_address(unchecked).await?;
        let fees_pre = generate_fee_premints(&self.wdc, &inputs, op.fees).await?;
        let fees_signatures = self.wdc.sign(fees_pre.blinded_messages()).await?;
        let keyset = self.wdc.keyset(fees_pre.keyset_id).await?;
        let fees = extract_proofs(fees_pre, fees_signatures.clone(), keyset)?;
        // update state
        self.wdc.burn(inputs.clone()).await?;
        let txid = match self
            .clowder_cl
            .melt_onchain(
                qid,
                op.target,
                recipient,
                inputs.clone(),
                fees_signatures,
                op.commitment,
            )
            .await
        {
            Ok(txid) => txid,
            Err(e) => {
                tracing::warn!(
                    "Failed to melt onchain for quote {qid}: {e}, recovering proofs {:?}",
                    op.input_ys,
                );
                self.wdc.recover(inputs.clone()).await?;
                return Err(Error::Internal(format!("Failed to melt onchain: {e}")));
            }
        };
        vault.store_proofs(fees).await?;
        let new = onchain::MeltStatus::Paid { tx: txid };
        match self.repo.update_meltop_status(qid, new).await {
            Ok(_) => {}
            Err(e) => {
                tracing::error!("DB Failure, lost MeltStatus update for {qid} with txid {txid:?}");
                return Err(e);
            }
        }
        let response = wire_melt::MeltOnchainResponse { txid };
        Ok(response)
    }

    pub async fn mint_onchain(
        &self,
        qid: Uuid,
        mint_id: secp256k1::PublicKey,
    ) -> Result<Vec<cashu::BlindSignature>> {
        let signatures = self.clowder_cl.fetch_mint_signatures(qid, mint_id).await?;
        Ok(signatures)
    }
}

fn cross_check_locked_fps(
    input_ys: Vec<cashu::PublicKey>,
    pending_ops: &[onchain::MeltOperation],
) -> Result<()> {
    assert!(!input_ys.is_empty());
    let input_set: HashSet<cashu::PublicKey> = HashSet::from_iter(input_ys);
    for op in pending_ops {
        let op_set: HashSet<cashu::PublicKey> = HashSet::from_iter(op.input_ys.clone());
        if !input_set.is_disjoint(&op_set) {
            return Err(Error::InvalidInput(String::from(
                "some proofs are locked in pending melt operations",
            )));
        }
    }
    Ok(())
}

async fn generate_fee_premints(
    wdc: &Arc<dyn WildcatClient>,
    inputs: &[cashu::Proof],
    fees: cashu::Amount,
) -> Result<cashu::PreMintSecrets> {
    assert!(!inputs.is_empty());
    let p1 = inputs.first().unwrap();
    let mut kid = p1.keyset_id;
    let mut expiry = wdc.keyset_info(kid).await?.final_expiry.unwrap_or_default();
    for p in inputs {
        if p.keyset_id == kid {
            continue;
        }
        let new_expiry = wdc
            .keyset_info(p.keyset_id)
            .await?
            .final_expiry
            .unwrap_or_default();
        if new_expiry > expiry {
            kid = p.keyset_id;
            expiry = new_expiry;
        }
    }
    let keyset = wdc.keyset(kid).await?;
    let pre = cashu::PreMintSecrets::random(
        kid,
        fees,
        &cashu::amount::SplitTarget::None,
        &bcr_wdc_utils::keys::to_fee_and_amounts(&keyset),
    )?;
    Ok(pre)
}

fn extract_proofs(
    premint: cashu::PreMintSecrets,
    signatures: Vec<cashu::BlindSignature>,
    keys: cashu::KeySet,
) -> Result<Vec<cashu::Proof>> {
    let (rs, secrets) = premint
        .secrets
        .into_iter()
        .map(|secret| (secret.r, secret.secret))
        .unzip();
    let prfs = cashu::dhke::construct_proofs(signatures, rs, secrets, &keys.keys)?;
    Ok(prfs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onchain::{MockClowderClient, MockRepository, MockVaultService, MockWildcatClient};
    use bcr_common::{core, core_tests};
    use bcr_wdc_utils::signatures::test_utils as signatures_test;
    use bitcoin::hashes::Hash;
    use cashu::Amount;
    use mockall::predicate::*;
    use std::str::FromStr;

    fn dummy_attestation() -> bcr_common::wire::attestation::IssuanceAttestation {
        let kp = core::generate_random_keypair();
        let signature = bitcoin::secp256k1::schnorr::Signature::from_slice(&[0; 64]).unwrap();
        bcr_common::wire::attestation::IssuanceAttestation {
            beta_id: kp.public_key(),
            fp_digest: [0u8; 32],
            coords_mac: [0u8; 32],
            signature,
        }
    }

    #[tokio::test]
    async fn new_onchain_mintop() {
        let mut wdc = MockWildcatClient::new();
        let mut repo = MockRepository::new();
        let mut clowder = MockClowderClient::new();
        let (info, keyset) = core_tests::generate_random_ecash_keyset();
        wdc.expect_get_active_keyset()
            .times(1)
            .returning(move || Ok(info.id));
        clowder
            .expect_request_onchain_mint_address()
            .times(1)
            .returning(|_, _| {
                Ok(
                    bitcoin::Address::from_str("1BwBExCU5qfkt1G7rqX8zDkKhhGe2p9Fdb")
                        .unwrap()
                        .assume_checked(),
                )
            });
        repo.expect_store_mintop().times(1).returning(|_| Ok(()));
        clowder
            .expect_sign_onchain_mint_response()
            .times(1)
            .returning(|_| {
                let signature =
                    bitcoin::secp256k1::schnorr::Signature::from_slice(&[0; 64]).unwrap();
                Ok((String::new(), signature))
            });
        let service = Service {
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            quote_expiry: chrono::Duration::seconds(3600),
            min_mint_threshold: bitcoin::Amount::ZERO,
            min_melt_threshold: bitcoin::Amount::ZERO,
            alpha_id: core::generate_random_keypair().public_key(),
        };
        let blinds: Vec<_> = signatures_test::generate_blinds(keyset.id, &[Amount::from(8_u64)])
            .into_iter()
            .map(|b| b.0)
            .collect();
        let request = wire_mint::OnchainMintQuoteRequest {
            blinded_messages: blinds,
            wallet_key: core::generate_random_keypair().public_key().into(),
        };
        service
            .create_onchain_mint_quote(request, chrono::Utc::now())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn new_onchain_mintop_blinds_less_than_threshold() {
        let wdc = MockWildcatClient::new();
        let repo = MockRepository::new();
        let clowder = MockClowderClient::new();
        let service = Service {
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            quote_expiry: chrono::Duration::seconds(3600),
            min_mint_threshold: bitcoin::Amount::from_sat(1000),
            min_melt_threshold: bitcoin::Amount::ZERO,
            alpha_id: core::generate_random_keypair().public_key(),
        };
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let blinds: Vec<_> = signatures_test::generate_blinds(keyset.id, &[Amount::from(8_u64)])
            .into_iter()
            .map(|b| b.0)
            .collect();
        let request = wire_mint::OnchainMintQuoteRequest {
            blinded_messages: blinds,
            wallet_key: core::generate_random_keypair().public_key().into(),
        };
        service
            .create_onchain_mint_quote(request, chrono::Utc::now())
            .await
            .unwrap_err();
    }

    #[tokio::test]
    async fn new_onchain_meltop_no_reserves() {
        let mut wdc = MockWildcatClient::new();
        let mut repo = MockRepository::new();
        let mut clowder = MockClowderClient::new();
        wdc.expect_verify_fingerprints()
            .times(1)
            .returning(|_| Ok(()));
        wdc.expect_check_spendable().times(1).returning(|ys| {
            let mut states = Vec::with_capacity(ys.len());
            for y in ys {
                states.push(cashu::ProofState {
                    y,
                    state: cashu::State::Unspent,
                    witness: None,
                });
            }
            Ok(states)
        });
        clowder
            .expect_get_onchain_reserve()
            .times(1)
            .returning(|| Ok(bitcoin::Amount::from_sat(100_000)));
        clowder
            .expect_verify_onchain_address()
            .times(1)
            .returning(|addr| Ok(addr.assume_checked()));
        clowder
            .expect_estimate_onchain_fees()
            .times(1)
            .returning(|_| Ok(bitcoin::Amount::from_sat(10)));
        let qid = Uuid::new_v4();
        let cloned_qid = qid;
        repo.expect_list_pending_meltops()
            .times(1)
            .returning(move |_| Ok(vec![cloned_qid]));
        repo.expect_load_meltop()
            .times(1)
            .with(eq(qid))
            .returning(move |_| {
                Ok(onchain::MeltOperation {
                    qid,
                    available: cashu::Amount::from(99_000),
                    address: String::new(),
                    target: bitcoin::Amount::from_sat(99_000),
                    expiry: chrono::Utc::now() + chrono::Duration::seconds(3600),
                    fees: cashu::Amount::ZERO,
                    wallet_key: core::generate_random_keypair().public_key().into(),
                    input_ys: vec![],
                    fp_digest: [0u8; 32],
                    commitment: bitcoin::secp256k1::schnorr::Signature::from_slice(&[0; 64])
                        .unwrap(),
                    status: onchain::MeltStatus::Pending,
                })
            });
        repo.expect_store_denied_meltop()
            .times(1)
            .returning(|_| Ok(()));
        clowder
            .expect_authenticate_attestation()
            .times(1)
            .returning(|_, _| Ok(()));
        let service = Service {
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            quote_expiry: chrono::Duration::seconds(3600),
            min_mint_threshold: bitcoin::Amount::ZERO,
            min_melt_threshold: bitcoin::Amount::ZERO,
            alpha_id: core::generate_random_keypair().public_key(),
        };
        let address = bitcoin::Address::from_str("1BwBExCU5qfkt1G7rqX8zDkKhhGe2p9Fdb").unwrap();
        let amounts = vec![Amount::from(512), Amount::from(512)];
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let fps = signatures_test::generate_fingerprints(&keyset, &amounts);
        let request = wire_melt::MeltQuoteOnchainRequest {
            inputs: bcr_common::wire::attestation::AttestedFingerprints {
                inputs: fps,
                attestation: dummy_attestation(),
            },
            address,
            wallet_key: core::generate_random_keypair().public_key().into(),
        };
        let response = service
            .create_onchain_melt_quote(request, chrono::Utc::now())
            .await;
        assert!(matches!(response, Err(Error::Unavailable(_))));
    }

    #[tokio::test]
    async fn new_onchain_meltop_ok() {
        let mut wdc = MockWildcatClient::new();
        let mut repo = MockRepository::new();
        let mut clowder = MockClowderClient::new();
        wdc.expect_verify_fingerprints()
            .times(1)
            .returning(|_| Ok(()));
        wdc.expect_check_spendable().times(1).returning(|ys| {
            let mut states = Vec::with_capacity(ys.len());
            for y in ys {
                states.push(cashu::ProofState {
                    y,
                    state: cashu::State::Unspent,
                    witness: None,
                });
            }
            Ok(states)
        });
        wdc.expect_reserve_inputs()
            .times(1)
            .returning(|_, _| Ok(()));
        clowder
            .expect_get_onchain_reserve()
            .times(1)
            .returning(|| Ok(bitcoin::Amount::from_sat(100_000)));
        clowder
            .expect_verify_onchain_address()
            .times(1)
            .returning(|addr| Ok(addr.assume_checked()));
        repo.expect_list_pending_meltops()
            .times(1)
            .returning(|_| Ok(vec![]));
        clowder
            .expect_sign_onchain_melt_response()
            .times(1)
            .returning(|_, _, _| {
                let signature =
                    bitcoin::secp256k1::schnorr::Signature::from_slice(&[0; 64]).unwrap();
                Ok((String::new(), signature))
            });
        clowder
            .expect_estimate_onchain_fees()
            .times(1)
            .returning(|_| Ok(bitcoin::Amount::from_sat(10)));
        repo.expect_store_meltop().times(1).returning(|_, _| Ok(()));
        clowder
            .expect_authenticate_attestation()
            .times(1)
            .returning(|_, _| Ok(()));
        let service = Service {
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            quote_expiry: chrono::Duration::seconds(3600),
            min_mint_threshold: bitcoin::Amount::ZERO,
            min_melt_threshold: bitcoin::Amount::ZERO,
            alpha_id: core::generate_random_keypair().public_key(),
        };
        let address = bitcoin::Address::from_str("1BwBExCU5qfkt1G7rqX8zDkKhhGe2p9Fdb").unwrap();
        let amounts = vec![Amount::from(512), Amount::from(512)];
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let fps = signatures_test::generate_fingerprints(&keyset, &amounts);
        let request = wire_melt::MeltQuoteOnchainRequest {
            inputs: bcr_common::wire::attestation::AttestedFingerprints {
                inputs: fps,
                attestation: dummy_attestation(),
            },
            address,
            wallet_key: core::generate_random_keypair().public_key().into(),
        };
        service
            .create_onchain_melt_quote(request, chrono::Utc::now())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn melt_onchain_ok() {
        let mut wdc = MockWildcatClient::new();
        let mut repo = MockRepository::new();
        let mut clowder = MockClowderClient::new();
        let mut vault = MockVaultService::new();
        let (kinfo, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &[Amount::from(8_u64)]);
        let input_ys = proofs.ys().unwrap();
        let qid = Uuid::new_v4();
        let signature = bitcoin::secp256k1::schnorr::Signature::from_slice(&[0; 64]).unwrap();
        let op = onchain::MeltOperation {
            qid,
            available: cashu::Amount::from(8),
            address: String::from("1BwBExCU5qfkt1G7rqX8zDkKhhGe2p9Fdb"),
            target: bitcoin::Amount::from_sat(8),
            expiry: chrono::Utc::now() + chrono::Duration::seconds(3600),
            fees: cashu::Amount::ZERO,
            wallet_key: core::generate_random_keypair().public_key().into(),
            input_ys,
            fp_digest: wire_attestation::fp_digest(
                &wire_attestation::project_to_fingerprints(&proofs).unwrap(),
            ),
            commitment: signature,
            status: onchain::MeltStatus::Pending,
        };
        wdc.expect_verify_proofs().times(1).returning(|_| Ok(()));
        wdc.expect_check_spendable().times(1).returning(|ys| {
            let mut states = Vec::with_capacity(ys.len());
            for y in ys {
                states.push(cashu::ProofState {
                    y,
                    state: cashu::State::Unspent,
                    witness: None,
                });
            }
            Ok(states)
        });
        repo.expect_load_meltop()
            .times(1)
            .returning(move |_| Ok(op.clone()));
        clowder
            .expect_verify_onchain_address()
            .times(1)
            .returning(|addr| Ok(addr.assume_checked()));
        wdc.expect_keyset_info()
            .times(1)
            .returning(move |_| Ok(cashu::KeySetInfo::from(kinfo.clone())));
        let cloned_keyset = keyset.clone();
        wdc.expect_keyset()
            .times(2)
            .returning(move |_| Ok(bcr_wdc_utils::keys::to_keyset(&cloned_keyset, None)));
        let cloned_keyset = keyset.clone();
        wdc.expect_sign().times(1).returning(move |blinds| {
            let amounts: Vec<_> = blinds.iter().map(|b| b.amount).collect();
            let signatures = signatures_test::generate_signatures(&cloned_keyset, &amounts);
            Ok(signatures)
        });
        vault.expect_store_proofs().times(1).returning(|_| Ok(()));
        wdc.expect_burn().times(1).returning(|_| Ok(()));
        clowder
            .expect_melt_onchain()
            .times(1)
            .returning(|_, _, _, _, _, _| Ok(bitcoin::Txid::all_zeros()));
        repo.expect_update_meltop_status()
            .times(1)
            .returning(|_, _| Ok(()));
        let service = Service {
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            quote_expiry: chrono::Duration::seconds(3600),
            min_mint_threshold: bitcoin::Amount::ZERO,
            min_melt_threshold: bitcoin::Amount::ZERO,
            alpha_id: core::generate_random_keypair().public_key(),
        };
        let request = wire_melt::MeltOnchainRequest {
            quote: qid,
            inputs: proofs,
        };
        service
            .melt_onchain(request, chrono::Utc::now(), &vault)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn melt_onchain_rejects_on_digest_mismatch() {
        let mut wdc = MockWildcatClient::new();
        let mut repo = MockRepository::new();
        let clowder = MockClowderClient::new();
        let vault = MockVaultService::new();
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &[Amount::from(8_u64)]);
        let input_ys = proofs.ys().unwrap();
        let qid = Uuid::new_v4();
        let signature = bitcoin::secp256k1::schnorr::Signature::from_slice(&[0; 64]).unwrap();
        // quote digest over different fingerprints than the presented proofs
        let op = onchain::MeltOperation {
            qid,
            available: cashu::Amount::from(8),
            address: String::from("1BwBExCU5qfkt1G7rqX8zDkKhhGe2p9Fdb"),
            target: bitcoin::Amount::from_sat(8),
            expiry: chrono::Utc::now() + chrono::Duration::seconds(3600),
            fees: cashu::Amount::ZERO,
            wallet_key: core::generate_random_keypair().public_key().into(),
            input_ys,
            fp_digest: [1u8; 32],
            commitment: signature,
            status: onchain::MeltStatus::Pending,
        };
        wdc.expect_verify_proofs().times(1).returning(|_| Ok(()));
        wdc.expect_check_spendable().times(1).returning(|ys| {
            let mut states = Vec::with_capacity(ys.len());
            for y in ys {
                states.push(cashu::ProofState {
                    y,
                    state: cashu::State::Unspent,
                    witness: None,
                });
            }
            Ok(states)
        });
        repo.expect_load_meltop()
            .times(1)
            .returning(move |_| Ok(op.clone()));
        let service = Service {
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            quote_expiry: chrono::Duration::seconds(3600),
            min_mint_threshold: bitcoin::Amount::ZERO,
            min_melt_threshold: bitcoin::Amount::ZERO,
            alpha_id: core::generate_random_keypair().public_key(),
        };
        let request = wire_melt::MeltOnchainRequest {
            quote: qid,
            inputs: proofs,
        };
        let err = service
            .melt_onchain(request, chrono::Utc::now(), &vault)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    #[tokio::test]
    async fn melt_quote_rejects_when_attestation_invalid() {
        let wdc = MockWildcatClient::new();
        let repo = MockRepository::new();
        let mut clowder = MockClowderClient::new();
        // attestation is authenticated at quote (commitment) time, before any other work
        clowder
            .expect_authenticate_attestation()
            .times(1)
            .returning(|_, _| {
                Err(Error::Attestation(
                    bcr_common::wire::attestation::AttestationError::DigestMismatch,
                ))
            });
        let service = Service {
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            quote_expiry: chrono::Duration::seconds(3600),
            min_mint_threshold: bitcoin::Amount::ZERO,
            min_melt_threshold: bitcoin::Amount::ZERO,
            alpha_id: core::generate_random_keypair().public_key(),
        };
        let address = bitcoin::Address::from_str("1BwBExCU5qfkt1G7rqX8zDkKhhGe2p9Fdb").unwrap();
        let amounts = vec![Amount::from(512)];
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let fps = signatures_test::generate_fingerprints(&keyset, &amounts);
        let request = wire_melt::MeltQuoteOnchainRequest {
            inputs: bcr_common::wire::attestation::AttestedFingerprints {
                inputs: fps,
                attestation: dummy_attestation(),
            },
            address,
            wallet_key: core::generate_random_keypair().public_key().into(),
        };
        let err = service
            .create_onchain_melt_quote(request, chrono::Utc::now())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::Attestation(bcr_common::wire::attestation::AttestationError::DigestMismatch)
        ));
    }

    #[tokio::test]
    async fn new_onchain_mintop_blinds_different_kids() {
        let mut wdc = MockWildcatClient::new();
        let repo = MockRepository::new();
        let clowder = MockClowderClient::new();
        let (info1, keyset1) = core_tests::generate_random_ecash_keyset();
        let (_, keyset2) = core_tests::generate_random_ecash_keyset();
        wdc.expect_get_active_keyset()
            .times(1)
            .returning(move || Ok(info1.id));
        let service = Service {
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            quote_expiry: chrono::Duration::seconds(3600),
            min_mint_threshold: bitcoin::Amount::ZERO,
            min_melt_threshold: bitcoin::Amount::ZERO,
            alpha_id: core::generate_random_keypair().public_key(),
        };
        let blinds1: Vec<_> = signatures_test::generate_blinds(keyset1.id, &[Amount::from(8_u64)])
            .into_iter()
            .map(|b| b.0)
            .collect();
        let blinds2: Vec<_> = signatures_test::generate_blinds(keyset2.id, &[Amount::from(8_u64)])
            .into_iter()
            .map(|b| b.0)
            .collect();
        let mut blinded_messages = Vec::new();
        blinded_messages.extend(blinds1);
        blinded_messages.extend(blinds2);
        let request = wire_mint::OnchainMintQuoteRequest {
            blinded_messages,
            wallet_key: core::generate_random_keypair().public_key().into(),
        };
        service
            .create_onchain_mint_quote(request, chrono::Utc::now())
            .await
            .unwrap_err();
    }

    #[tokio::test]
    // test that melt_onchain succeeds when proofs are reserved
    async fn melt_onchain_proofs_reserved() {
        let mut wdc = MockWildcatClient::new();
        let mut repo = MockRepository::new();
        let mut clowder = MockClowderClient::new();
        let mut vault = MockVaultService::new();
        let (kinfo, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &[Amount::from(8_u64)]);
        let input_ys = proofs.ys().unwrap();
        let qid = Uuid::new_v4();
        let signature = bitcoin::secp256k1::schnorr::Signature::from_slice(&[0; 64]).unwrap();
        let op = onchain::MeltOperation {
            qid,
            available: cashu::Amount::from(8),
            address: String::from("1BwBExCU5qfkt1G7rqX8zDkKhhGe2p9Fdb"),
            target: bitcoin::Amount::from_sat(8),
            expiry: chrono::Utc::now() + chrono::Duration::seconds(3600),
            fees: cashu::Amount::ZERO,
            wallet_key: core::generate_random_keypair().public_key().into(),
            input_ys,
            fp_digest: wire_attestation::fp_digest(
                &wire_attestation::project_to_fingerprints(&proofs).unwrap(),
            ),
            commitment: signature,
            status: onchain::MeltStatus::Pending,
        };
        wdc.expect_verify_proofs().times(1).returning(|_| Ok(()));
        wdc.expect_check_spendable().times(1).returning(|ys| {
            let mut states = Vec::with_capacity(ys.len());
            for y in ys {
                states.push(cashu::ProofState {
                    y,
                    state: cashu::State::Reserved,
                    witness: None,
                });
            }
            Ok(states)
        });
        repo.expect_load_meltop()
            .times(1)
            .returning(move |_| Ok(op.clone()));
        clowder
            .expect_verify_onchain_address()
            .times(1)
            .returning(|addr| Ok(addr.assume_checked()));
        wdc.expect_keyset_info()
            .times(1)
            .returning(move |_| Ok(cashu::KeySetInfo::from(kinfo.clone())));
        let cloned_keyset = bcr_common::core::keys::to_keyset(&keyset, Some(true));
        wdc.expect_keyset()
            .times(2)
            .returning(move |_| Ok(cloned_keyset.clone()));
        wdc.expect_sign().times(1).returning(move |blinds| {
            let amounts: Vec<_> = blinds.iter().map(|b| b.amount).collect();
            let signatures = signatures_test::generate_signatures(&keyset, &amounts);
            Ok(signatures)
        });
        wdc.expect_burn()
            .with(eq(proofs.clone()))
            .times(1)
            .returning(|_| Ok(()));
        clowder
            .expect_melt_onchain()
            .times(1)
            .with(
                always(),
                always(),
                always(),
                eq(proofs.clone()),
                always(),
                always(),
            )
            .returning(|_, _, _, _, _, _| Ok(bitcoin::Txid::all_zeros()));
        vault.expect_store_proofs().times(1).returning(|_| Ok(()));
        repo.expect_update_meltop_status()
            .times(1)
            .returning(|_, _| Ok(()));
        let service = Service {
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            quote_expiry: chrono::Duration::seconds(3600),
            min_mint_threshold: bitcoin::Amount::ZERO,
            min_melt_threshold: bitcoin::Amount::ZERO,
            alpha_id: core::generate_random_keypair().public_key(),
        };
        let request = wire_melt::MeltOnchainRequest {
            quote: qid,
            inputs: proofs,
        };
        service
            .melt_onchain(request, chrono::Utc::now(), &vault)
            .await
            .unwrap();
    }
}
