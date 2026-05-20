// ----- standard library imports
use std::{str::FromStr, sync::Arc};
// ----- extra library imports
use bcr_common::{
    cashu::{self, ProofsMethods},
    wire::{melt as wire_melt, mint as wire_mint},
};
use bitcoin::secp256k1::PublicKey;
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    onchain::{self, ClowderClient, MintStatus, OnChainMintOperation, Repository, WildcatClient},
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
        let mintop = OnChainMintOperation {
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
        let core_fps: Vec<_> = inputs
            .iter()
            .cloned()
            .map(bcr_common::core::signature::ProofFingerprint::from)
            .collect();
        bcr_wdc_utils::signatures::basic_fingerprints_checks(&core_fps)?;
        self.wdc.verify_fingerprints(&inputs).await?;
        // Unspent fingerprints
        let mut input_ys = inputs.iter().map(|fp| fp.y).collect::<Vec<_>>();
        let states = self.wdc.check_spendable(input_ys.clone()).await?;
        let all_unspent = states
            .iter()
            .all(|s| matches!(s.state, cashu::State::Unspent));
        if !all_unspent {
            return Err(Error::InvalidInput(String::from("proofs already spent")));
        }
        // amount > dust
        let input_total = bitcoin::Amount::from_sat(inputs.iter().map(|fp| fp.amount).sum::<u64>());
        if input_total < self.min_melt_threshold {
            return Err(Error::InvalidInput(String::from("melt amount too low")));
        }
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
        if input_total < admin_fees + network_fees {
            return Err(Error::InvalidInput(String::from(
                "insufficient funds to cover fees",
            )));
        }
        // insufficient funds in clowder
        if input_total > reserve {
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
        let (content, commitment) = self.clowder_cl.sign_onchain_melt_response(&body).await?;
        input_ys.sort();
        let op = onchain::OnchainMeltOperation {
            qid,
            available: cashu::Amount::from(input_total.to_sat()),
            address: checked_address.to_string(),
            target,
            fees: cashu::Amount::from(admin_fees.to_sat()),
            expiry,
            wallet_key,
            input_ys,
            commitment,
            status: onchain::MeltStatus::Pending,
        };
        self.repo.store_meltop(op).await?;
        Ok(wire_melt::MeltQuoteOnchainResponse {
            content,
            commitment,
        })
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

    pub async fn melt_onchain(
        &self,
        request: wire_melt::MeltOnchainRequest,
        now: TStamp,
        vault: &dyn onchain::VaultService,
    ) -> Result<wire_melt::MeltOnchainResponse> {
        let wire_melt::MeltOnchainRequest {
            inputs,
            attestation,
            ..
        } = request;
        // verify proofs
        bcr_wdc_utils::signatures::basic_proofs_checks(&inputs)?;
        self.wdc.verify_proofs(&inputs).await?;
        // verify unspent
        let mut p_ys = inputs.ys()?;
        let states = self.wdc.check_spendable(p_ys.clone()).await?;
        let all_unspent = states
            .iter()
            .all(|s| matches!(s.state, cashu::State::Unspent));
        if !all_unspent {
            return Err(Error::InvalidInput(String::from("proofs already spent")));
        }
        // load melt operation
        let qid = request.quote;
        let op = self.repo.load_meltop(qid).await?;
        if now > op.expiry {
            return Err(Error::InvalidInput(String::from("Melt quote has expired")));
        }
        p_ys.sort();
        if p_ys != op.input_ys {
            return Err(Error::InvalidInput(String::from(
                "melt proofs != committed fingerprints",
            )));
        }
        // verify Beta-issued attestation before burning any proofs
        self.clowder_cl
            .verify_attestation(&self.alpha_id, &inputs, &attestation)
            .await?;
        let unchecked = bitcoin::Address::from_str(&op.address)?;
        let recipient = self.clowder_cl.verify_onchain_address(unchecked).await?;
        let fees_pre = generate_fee_premints(&self.wdc, &inputs, op.fees).await?;
        let fees_signatures = self.wdc.sign(fees_pre.blinded_messages()).await?;
        let keyset = self.wdc.keyset(fees_pre.keyset_id).await?;
        let fees = extract_proofs(fees_pre, fees_signatures.clone(), keyset)?;
        // update state
        self.wdc.burn(inputs.clone()).await?;
        let txs = match self
            .clowder_cl
            .melt_onchain(
                qid,
                op.target,
                recipient,
                inputs.clone(),
                fees_signatures,
                op.commitment,
                attestation,
            )
            .await
        {
            Ok(txs) => txs,
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
        let new = onchain::MeltStatus::Paid { tx: txs.clone() };
        match self.repo.update_meltop_status(qid, new).await {
            Ok(_) => {}
            Err(e) => {
                tracing::error!("DB Failure, lost MeltStatus update for {qid} with txs {txs:?}");
                return Err(e);
            }
        }
        let response = wire_melt::MeltOnchainResponse { txid: txs };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onchain::{MockClowderClient, MockRepository, MockVaultService, MockWildcatClient};
    use bcr_common::{core, core_tests};
    use bcr_wdc_utils::signatures::test_utils as signatures_test;
    use cashu::Amount;
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
        clowder
            .expect_get_onchain_reserve()
            .times(1)
            .returning(|| Ok(bitcoin::Amount::from_sat(100_000)));
        clowder
            .expect_verify_onchain_address()
            .times(1)
            .returning(|addr| Ok(addr.assume_checked()));
        clowder
            .expect_sign_onchain_melt_response()
            .times(1)
            .returning(|_| {
                let signature =
                    bitcoin::secp256k1::schnorr::Signature::from_slice(&[0; 64]).unwrap();
                Ok((String::new(), signature))
            });
        clowder
            .expect_estimate_onchain_fees()
            .times(1)
            .returning(|_| Ok(bitcoin::Amount::from_sat(10)));
        repo.expect_store_meltop().times(1).returning(|_| Ok(()));
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
            inputs: fps,
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
        let op = onchain::OnchainMeltOperation {
            qid,
            available: cashu::Amount::from(8),
            address: String::from("1BwBExCU5qfkt1G7rqX8zDkKhhGe2p9Fdb"),
            target: bitcoin::Amount::from_sat(8),
            expiry: chrono::Utc::now() + chrono::Duration::seconds(3600),
            fees: cashu::Amount::ZERO,
            wallet_key: core::generate_random_keypair().public_key().into(),
            input_ys,
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
            .expect_verify_attestation()
            .times(1)
            .returning(|_, _, _| Ok(()));
        clowder
            .expect_verify_onchain_address()
            .times(1)
            .returning(|addr| Ok(addr.assume_checked()));
        wdc.expect_keyset_info()
            .times(1)
            .returning(move |_| Ok(cashu::KeySetInfo::from(kinfo.clone())));
        let cloned_keyset = keyset.clone();
        wdc.expect_keyset()
            .times(1)
            .returning(move |_| Ok(cashu::KeySet::from(cloned_keyset.clone())));
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
            .returning(|_, _, _, _, _, _, _| {
                Ok(wire_melt::MeltTx {
                    alpha_txid: None,
                    beta_txid: None,
                })
            });
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
            attestation: dummy_attestation(),
        };
        service
            .melt_onchain(request, chrono::Utc::now(), &vault)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn melt_onchain_rejects_when_attestation_invalid() {
        let mut wdc = MockWildcatClient::new();
        let mut repo = MockRepository::new();
        let mut clowder = MockClowderClient::new();
        let vault = MockVaultService::new();
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &[Amount::from(8_u64)]);
        let input_ys = proofs.ys().unwrap();
        let qid = Uuid::new_v4();
        let signature = bitcoin::secp256k1::schnorr::Signature::from_slice(&[0; 64]).unwrap();
        let op = onchain::OnchainMeltOperation {
            qid,
            available: cashu::Amount::from(8),
            address: String::from("1BwBExCU5qfkt1G7rqX8zDkKhhGe2p9Fdb"),
            target: bitcoin::Amount::from_sat(8),
            expiry: chrono::Utc::now() + chrono::Duration::seconds(3600),
            fees: cashu::Amount::ZERO,
            wallet_key: core::generate_random_keypair().public_key().into(),
            input_ys,
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
            .expect_verify_attestation()
            .times(1)
            .returning(|_, _, _| {
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
        let request = wire_melt::MeltOnchainRequest {
            quote: qid,
            inputs: proofs,
            attestation: dummy_attestation(),
        };
        let err = service
            .melt_onchain(request, chrono::Utc::now(), &vault)
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
}

async fn generate_fee_premints(
    wdc: &Arc<dyn WildcatClient>,
    inputs: &[cashu::Proof],
    fees: cashu::Amount,
) -> Result<cashu::PreMintSecrets> {
    assert!(!inputs.is_empty());
    let p1 = inputs.first().unwrap();
    let mut kid = p1.keyset_id;
    let expiry = wdc.keyset_info(kid).await?.final_expiry.unwrap_or_default();
    for p in inputs {
        if p.keyset_id == kid {
            continue;
        }
        let info = wdc.keyset_info(p.keyset_id).await?;
        if info.final_expiry.unwrap_or_default() > expiry {
            kid = p.keyset_id;
        }
    }
    let pre = cashu::PreMintSecrets::random(kid, fees, &cashu::amount::SplitTarget::None)?;
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
