// ----- standard library imports
use std::{str::FromStr, sync::Arc};
// ----- extra library imports
use bcr_common::{
    cashu::{self, ProofsMethods},
    core::BillId,
    wire::{melt as wire_melt, mint as wire_mint},
};
use uuid::Uuid;
// ----- local imports
use crate::{
    debit::{self, ClowderClient, MintStatus, OnChainMintOperation, Repository, WildcatClient},
    error::{Error, Result},
    TStamp,
};

// ----- end imports

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MintQuote {
    pub qid: String,
    pub ebill_id: BillId,
    pub clowder_qid: uuid::Uuid,
    pub mint_complete: bool,
}

pub struct Service {
    pub wdc: Arc<dyn WildcatClient>,
    pub repo: Arc<dyn Repository>,
    pub clowder_cl: Arc<dyn ClowderClient>,
    pub quote_expiry: chrono::Duration,
    pub min_mint_threshold: bitcoin::Amount,
    pub min_melt_threshold: bitcoin::Amount,
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
        self.repo.store_onchain_mintop(mintop).await?;
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
        if request.amount < self.min_melt_threshold {
            return Err(Error::InvalidInput(String::from("melt amount too low")));
        }
        let address = self
            .clowder_cl
            .verify_onchain_address(request.address.clone())
            .await?;
        let input_sats: u64 = request.inputs.iter().map(|fp| fp.amount).sum();
        let total = cashu::Amount::from(input_sats);
        let expiry = now + self.quote_expiry;
        let qid = Uuid::new_v4();
        let body = wire_melt::MeltQuoteOnchainResponseBody {
            quote: qid,
            inputs: request.inputs.clone(),
            address: request.address.clone(),
            amount: request.amount,
            total,
            expiry: expiry.timestamp().max(0) as u64,
            wallet_key: request.wallet_key,
        };
        let (content, commitment) = self.clowder_cl.sign_onchain_melt_response(&body).await?;
        let op = debit::OnchainMeltOperation {
            qid,
            address: address.to_string(),
            amount: request.amount,
            expiry,
            fees: bitcoin::Amount::ZERO,
            wallet_key: request.wallet_key,
            commitment,
            status: debit::MeltStatus::Pending,
        };
        self.repo.store_onchain_meltop(op).await?;
        Ok(wire_melt::MeltQuoteOnchainResponse {
            content,
            commitment,
        })
    }

    pub async fn melt_onchain(
        &self,
        request: wire_melt::MeltOnchainRequest,
        now: TStamp,
    ) -> Result<wire_melt::MeltOnchainResponse> {
        let qid = request.quote;
        let op = self.repo.load_onchain_meltop(qid).await?;
        if now > op.expiry {
            return Err(Error::InvalidInput(String::from("Melt quote has expired")));
        }
        let proofs = request.inputs.clone();
        let inputs_amount = proofs.total_amount()?;
        let req_camount = cashu::Amount::from((op.amount + op.fees).to_sat());
        if inputs_amount < req_camount {
            return Err(Error::InvalidInput(format!(
                "input amount, required: {req_camount}, provided: {inputs_amount}"
            )));
        }
        let unchecked = bitcoin::Address::from_str(&op.address)?;
        let recipient = self.clowder_cl.verify_onchain_address(unchecked).await?;
        self.wdc.burn(proofs.clone()).await?;
        let txs = match self
            .clowder_cl
            .melt_onchain(qid, op.amount, recipient, proofs, op.commitment)
            .await
        {
            Ok(txs) => txs,
            Err(e) => {
                let ys = request.inputs.ys()?;
                tracing::warn!(
                    "Failed to melt onchain for quote {qid}: {e}, recovering proofs {:?}",
                    ys
                );
                self.wdc.recover(request.inputs.clone()).await?;
                return Err(Error::Internal(format!("Failed to melt onchain: {e}")));
            }
        };
        let new = debit::MeltStatus::Paid { tx: txs.clone() };
        match self.repo.update_onchain_meltop_status(qid, new).await {
            Ok(_) => {}
            Err(e) => {
                tracing::error!("DB Failure, lost MeltStatus update for {qid} with txs {txs:?}");
                return Err(e);
            }
        }
        let response = wire_melt::MeltOnchainResponse { txid: txs };
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debit::{MockClowderClient, MockRepository, MockWildcatClient};
    use bcr_common::core_tests;
    use bcr_wdc_utils::signatures::test_utils as signatures_test;
    use cashu::Amount;
    use std::str::FromStr;

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
        repo.expect_store_onchain_mintop()
            .times(1)
            .returning(|_| Ok(()));
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
        };
        let blinds: Vec<_> = signatures_test::generate_blinds(keyset.id, &[Amount::from(8_u64)])
            .into_iter()
            .map(|b| b.0)
            .collect();
        let request = wire_mint::OnchainMintQuoteRequest {
            blinded_messages: blinds,
            wallet_key: core_tests::generate_random_keypair().public_key().into(),
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
        };
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let blinds: Vec<_> = signatures_test::generate_blinds(keyset.id, &[Amount::from(8_u64)])
            .into_iter()
            .map(|b| b.0)
            .collect();
        let request = wire_mint::OnchainMintQuoteRequest {
            blinded_messages: blinds,
            wallet_key: core_tests::generate_random_keypair().public_key().into(),
        };
        service
            .create_onchain_mint_quote(request, chrono::Utc::now())
            .await
            .unwrap_err();
    }

    #[tokio::test]
    async fn new_onchain_meltop_ok() {
        let wdc = MockWildcatClient::new();
        let mut repo = MockRepository::new();
        let mut clowder = MockClowderClient::new();
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
        repo.expect_store_onchain_meltop()
            .times(1)
            .returning(|_| Ok(()));
        let service = Service {
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            quote_expiry: chrono::Duration::seconds(3600),
            min_mint_threshold: bitcoin::Amount::ZERO,
            min_melt_threshold: bitcoin::Amount::ZERO,
        };
        let address = bitcoin::Address::from_str("1BwBExCU5qfkt1G7rqX8zDkKhhGe2p9Fdb").unwrap();
        let request = wire_melt::MeltQuoteOnchainRequest {
            inputs: Vec::new(),
            address,
            amount: bitcoin::Amount::from_sat(1000),
            wallet_key: core_tests::generate_random_keypair().public_key().into(),
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
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, &[Amount::from(8_u64)]);
        let qid = Uuid::new_v4();
        let signature = bitcoin::secp256k1::schnorr::Signature::from_slice(&[0; 64]).unwrap();
        let op = debit::OnchainMeltOperation {
            qid,
            address: String::from("1BwBExCU5qfkt1G7rqX8zDkKhhGe2p9Fdb"),
            amount: bitcoin::Amount::from_sat(8),
            expiry: chrono::Utc::now() + chrono::Duration::seconds(3600),
            fees: bitcoin::Amount::ZERO,
            wallet_key: core_tests::generate_random_keypair().public_key().into(),
            commitment: signature,
            status: debit::MeltStatus::Pending,
        };
        repo.expect_load_onchain_meltop()
            .times(1)
            .returning(move |_| Ok(op.clone()));
        clowder
            .expect_verify_onchain_address()
            .times(1)
            .returning(|addr| Ok(addr.assume_checked()));
        wdc.expect_burn().times(1).returning(|_| Ok(()));
        clowder
            .expect_melt_onchain()
            .times(1)
            .returning(|_, _, _, _, _| {
                Ok(wire_melt::MeltTx {
                    alpha_txid: None,
                    beta_txid: None,
                })
            });
        repo.expect_update_onchain_meltop_status()
            .times(1)
            .returning(|_, _| Ok(()));
        let service = Service {
            wdc: Arc::new(wdc),
            repo: Arc::new(repo),
            clowder_cl: Arc::new(clowder),
            quote_expiry: chrono::Duration::seconds(3600),
            min_mint_threshold: bitcoin::Amount::ZERO,
            min_melt_threshold: bitcoin::Amount::ZERO,
        };
        let request = wire_melt::MeltOnchainRequest {
            quote: qid,
            inputs: proofs,
        };
        service
            .melt_onchain(request, chrono::Utc::now())
            .await
            .unwrap();
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
            wallet_key: core_tests::generate_random_keypair().public_key().into(),
        };
        service
            .create_onchain_mint_quote(request, chrono::Utc::now())
            .await
            .unwrap_err();
    }
}
