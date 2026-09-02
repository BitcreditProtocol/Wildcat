// ----- standard library imports
#[cfg(test)]
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    core::{BillId, NodeId},
    wire::{
        bill as wire_bill,
        quotes::{self as wire_quotes, SignedCreditQuoteReissuePermit},
    },
};
use bitcoin as btc;
use uuid::Uuid;
// ----- local imports
use crate::{
    authorization::{
        denial_result_digest, offer_result_digest, ApplicantActionCommandValue,
        AuthorizationVerifier, SignedCreditApplicantActionCommandV1,
        SignedCreditQuoteDenialCommandV1,
    },
    error::{Error, Result},
    persistence::{ApplicantActionProjectionMutation, GovernedDenialInput, Repository},
    quotes::{BillInfo, CreditProgramBinding, LightQuote, Quote, Status, StatusDiscriminants},
    TStamp,
};

// ----- end imports

// ---------- required traits
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ListFilters {
    pub bill_maturity_date_from: Option<chrono::NaiveDate>,
    pub bill_maturity_date_to: Option<chrono::NaiveDate>,
    pub status: Option<StatusDiscriminants>,
    pub bill_id: Option<BillId>,
    pub bill_drawee_id: Option<NodeId>,
    pub bill_drawer_id: Option<NodeId>,
    pub bill_payer_id: Option<NodeId>,
    pub bill_holder_id: Option<NodeId>,
}

#[derive(Debug, Clone)]
pub enum SortOrder {
    BillMaturityDateAsc,
    BillMaturityDateDesc,
    SubmittedDesc,
    SubmittedAsc,
}

pub enum MintingStatus {
    Disabled,
    Enabled(cashu::Amount),
}
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait WdcClient: Send + Sync {
    async fn get_keyset_with_expiration_date(
        &self,
        expiration_date: chrono::NaiveDate,
    ) -> Result<cashu::Id>;
    async fn get_keys(&self, keyset_id: cashu::Id) -> Result<cashu::KeySet>;
    async fn add_new_mint_operation(
        &self,
        qid: Uuid,
        kid: cashu::Id,
        pk: cashu::PublicKey,
        target: cashu::Amount,
        bill_id: BillId,
    ) -> Result<()>;
    async fn sign(&self, msgs: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>>;
    async fn get_minting_status(&self, qid: Uuid) -> Result<MintingStatus>;
    async fn validate_and_decrypt_shared_bill(
        &self,
        shared_bill: &wire_quotes::SharedBill,
    ) -> Result<wire_quotes::BillInfo>;
    async fn validate_endorsed_bill_matches_shared_bill(
        &self,
        bill_id: BillId,
        shared_bill_data: String,
    ) -> Result<bool>;
    async fn get_shared_ebill_history(
        &self,
        bill_id: BillId,
        shared_bill_data: String,
    ) -> Result<Vec<wire_bill::BillHistoryBlock>>;
    async fn get_ebill(&self, bid: BillId) -> Result<wire_bill::BitcreditBill>;
    async fn collect_fees(&self, proofs: Vec<cashu::Proof>) -> Result<()>;
}

// ---------- Service
pub struct Service {
    pub wdc_client: Box<dyn WdcClient + Send + Sync>,
    pub quotes: Box<dyn Repository + Send + Sync>,
    pub mint_url: cashu::MintUrl,
    pub credit_program: CreditProgramBinding,
    pub(crate) authorization_verifier: AuthorizationVerifier,
}

impl Service {
    pub(crate) const USER_DECISION_RETENTION: chrono::Duration = chrono::Duration::days(1);

    async fn _lookup(&self, qid: uuid::Uuid, now: TStamp) -> Result<Quote> {
        let mut quote = self
            .quotes
            .load(qid)
            .await?
            .ok_or(Error::ResourceNotFound(qid.to_string()))?;
        let changed = quote.check_expire(now);
        if changed {
            self.quotes
                .update_status_if_offered(quote.id, quote.status.clone())
                .await?;
        }
        Ok(quote)
    }

    async fn new_quote(
        &self,
        bill: BillInfo,
        minting_pub_key: cashu::PublicKey,
        submitted: TStamp,
        expected_latest: Option<Uuid>,
    ) -> Result<Uuid> {
        let quote = Quote::new(
            bill,
            minting_pub_key,
            submitted,
            self.credit_program.clone(),
        );
        self.quotes.store_if_latest(expected_latest, quote).await
    }

    pub async fn validate_and_decrypt_shared_bill(
        &self,
        shared_bill: &wire_quotes::SharedBill,
    ) -> Result<wire_quotes::BillInfo> {
        self.wdc_client
            .validate_and_decrypt_shared_bill(shared_bill)
            .await
    }

    pub async fn enquire(
        &self,
        bill: BillInfo,
        pub_key: cashu::PublicKey,
        submitted: TStamp,
    ) -> Result<uuid::Uuid> {
        validate_basic_ebill_rules(&bill, chrono::Utc::now().date_naive())?;
        let holder_id = &bill.endorsees.last().unwrap_or(&bill.payee).node_id();
        let mut quotes = self.quotes.search_by_bill(&bill.id, holder_id).await?;

        // pick the more recent quote for this eBill/endorser
        quotes.sort_by_key(|q| (q.submitted, q.id));
        if let Some(last) = quotes.last_mut() {
            let changed = last.check_expire(submitted);
            if changed {
                self.quotes
                    .update_status_if_offered(last.id, last.status.clone())
                    .await?;
            }
        }
        match quotes.last() {
            // A governed denial is not an ordinary user-decision timeout. It remains the latest
            // quote until the reviewed-correction endpoint consumes a signed reissue permit that
            // binds the old denial, corrected case, current holder, and preselected new quote id.
            // Letting the legacy enquiry path create a fresh quote after one day would bypass that
            // governed reassessment boundary.
            Some(Quote {
                id,
                status: Status::Denied { .. },
                authorization_receipt: Some(receipt),
                ..
            }) if receipt.action == crate::authorization::QUOTE_DENIAL_ACTION => Ok(*id),
            Some(Quote {
                id,
                status: Status::Canceled { tstamp },
                ..
            })
            | Some(Quote {
                id,
                status: Status::Denied { tstamp },
                ..
            })
            | Some(Quote {
                id,
                status: Status::OfferExpired { tstamp, .. },
                ..
            })
            | Some(Quote {
                id,
                status: Status::Rejected { tstamp, .. },
                ..
            }) => {
                if (submitted - tstamp) > Self::USER_DECISION_RETENTION {
                    self.new_quote(bill, pub_key, submitted, Some(*id)).await
                } else {
                    Ok(*id)
                }
            }
            Some(Quote {
                id,
                status: Status::Pending { .. },
                ..
            })
            | Some(Quote {
                id,
                status: Status::Offered { .. },
                ..
            })
            | Some(Quote {
                id,
                status: Status::Accepted { .. },
                ..
            })
            | Some(Quote {
                id,
                status: Status::FailedEbillValidation { .. },
                ..
            }) => Ok(*id),
            Some(Quote {
                id,
                status: Status::MintingEnabled { .. },
                ..
            }) => Ok(*id),
            None => self.new_quote(bill, pub_key, submitted, None).await,
        }
    }

    /**
     * Creates one fresh Pending quote only when AI authority and the current holder's separately
     * signed request name the same corrected terminal case. The repository owns one-use and replay
     * semantics; the legacy enquiry path above intentionally never inspects this permit.
     */
    pub async fn reissue_enquire(
        &self,
        bill: BillInfo,
        pub_key: cashu::PublicKey,
        signed: SignedCreditQuoteReissuePermit,
        submitted: TStamp,
    ) -> Result<uuid::Uuid> {
        let previous_id = signed.permit.previous_mint_quote_id;
        let previous = self
            .quotes
            .load(previous_id)
            .await?
            .ok_or_else(|| Error::ResourceNotFound(previous_id.to_string()))?;
        let verified = self
            .authorization_verifier
            .verify_quote_reissue(signed, &previous, submitted)?;
        if !same_reissue_bill(&previous.bill, &bill) {
            return Err(Error::CreditQuoteReissueConflict);
        }
        let credit_program = previous
            .credit_program()
            .cloned()
            .ok_or(Error::CreditProgramNotBound(previous.id))?;
        let quote = Quote::new_with_id(
            verified.signed.permit.reissued_mint_quote_id,
            bill,
            pub_key,
            submitted,
            credit_program,
        );
        self.quotes
            .execute_quote_reissue(verified.signed, quote, submitted)
            .await
    }

    pub async fn cancel(&self, id: uuid::Uuid, submitted: TStamp) -> Result<()> {
        let old = self.quotes.load(id).await?;
        if old.is_none() {
            return Err(Error::ResourceNotFound(id.to_string()));
        }
        let mut quote = old.unwrap();
        quote.cancel(submitted)?;
        self.quotes
            .update_status_if_pending(quote.id, quote.status)
            .await?;
        Ok(())
    }

    pub async fn deny(&self, id: uuid::Uuid, submitted: TStamp) -> Result<()> {
        let old = self.quotes.load(id).await?;
        if old.is_none() {
            return Err(Error::ResourceNotFound(id.to_string()));
        }
        let mut quote = old.unwrap();
        quote.deny(submitted)?;
        self.quotes
            .update_status_if_pending(quote.id, quote.status)
            .await?;
        Ok(())
    }

    pub async fn deny_governed(
        &self,
        signed: SignedCreditQuoteDenialCommandV1,
        now: TStamp,
    ) -> Result<wire_quotes::CreditAuthorizationReceipt> {
        let qid = uuid::Uuid::parse_str(&signed.command.mint_quote_id)
            .map_err(|_| Error::CreditQuoteDenialInvalid)?;
        let quote = self
            .quotes
            .load(qid)
            .await?
            .ok_or_else(|| Error::ResourceNotFound(qid.to_string()))?;
        let verified = self
            .authorization_verifier
            .verify_quote_denial(signed, &quote, now)?;
        let denied_at = chrono::DateTime::from_timestamp_millis(now.timestamp_millis())
            .ok_or(Error::CreditQuoteDenialInvalid)?;
        let completed_at = denied_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let receipt = wire_quotes::CreditAuthorizationReceipt {
            receipt_version: String::from("credit-authorization-receipt-v1"),
            operation_id: verified.operation_id,
            authorization_digest: verified.command_digest,
            case_id: verified.command.case_id,
            status: String::from("completed"),
            mint_id: verified.command.mint_id,
            bill_id: verified.command.bill_id,
            action: verified.command.action,
            effect_id: qid.to_string(),
            result_digest: denial_result_digest(qid, &completed_at),
            completed_at,
            synthetic: true,
        };
        self.quotes
            .execute_governed_denial(GovernedDenialInput {
                quote_id: qid,
                receipt,
                denied_at,
                expires_at: verified.expires_at,
            })
            .await
    }

    pub async fn apply_applicant_action_projection(
        &self,
        signed: SignedCreditApplicantActionCommandV1,
        now: TStamp,
    ) -> Result<wire_quotes::CreditApplicantActionReceipt> {
        let qid = uuid::Uuid::parse_str(&signed.command.mint_quote_id)
            .map_err(|_| Error::ApplicantActionProjectionInvalid)?;
        let quote = self
            .quotes
            .load(qid)
            .await?
            .ok_or_else(|| Error::ResourceNotFound(qid.to_string()))?;
        let verified = self
            .authorization_verifier
            .verify_applicant_action_projection(signed, &quote, now)?;
        let projection = match verified.command.applicant_action {
            ApplicantActionCommandValue::ClarificationRequired => {
                Some(wire_quotes::ApplicantActionProjection {
                    kind: wire_quotes::ApplicantActionKind::Clarification,
                    revision_digest: verified.command.revision_digest.clone(),
                })
            }
            ApplicantActionCommandValue::None => None,
        };
        let receipt = wire_quotes::CreditApplicantActionReceipt {
            schema_version: String::from("credit-applicant-action-receipt-v1"),
            operation_id: verified.command.operation_id.clone(),
            mint_quote_id: qid,
            credit_program_version: verified.command.credit_program_version.clone(),
            credit_program_digest: verified.command.credit_program_digest.clone(),
            revision_digest: verified.command.revision_digest.clone(),
            expected_revision_digest: verified.command.expected_revision_digest.clone(),
            applicant_action: verified.command.applicant_action,
            action: verified.command.action.clone(),
            status: String::from("completed"),
            completed_at: now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        };
        self.quotes
            .apply_applicant_action_projection(ApplicantActionProjectionMutation {
                quote_id: qid,
                expected_revision_digest: verified.command.expected_revision_digest,
                revision_digest: verified.command.revision_digest,
                projection,
                operation_id: verified.command.operation_id,
                command_digest: verified.command_digest,
                applied_at: now,
                expires_at: verified.expires_at,
                receipt,
            })
            .await
    }

    pub async fn reject(&self, id: uuid::Uuid, tstamp: TStamp) -> Result<()> {
        let old = self.quotes.load(id).await?;
        if old.is_none() {
            return Err(Error::ResourceNotFound(id.to_string()));
        }
        let mut quote = old.unwrap();
        quote.reject(tstamp)?;
        self.quotes
            .update_status_if_offered(quote.id, quote.status)
            .await?;
        Ok(())
    }

    pub async fn accept(&self, id: uuid::Uuid, submitted: TStamp) -> Result<()> {
        let old = self.quotes.load(id).await?;
        if old.is_none() {
            return Err(Error::ResourceNotFound(id.to_string()));
        }
        let mut quote = old.unwrap();
        quote.accept(submitted)?;
        self.quotes
            .update_status_if_offered(quote.id, quote.status)
            .await?;
        Ok(())
    }

    pub async fn lookup(&self, qid: uuid::Uuid, now: TStamp) -> Result<Quote> {
        let quote = self._lookup(qid, now).await?;
        Ok(quote)
    }

    pub async fn lookup_applicant_action_projection(
        &self,
        qid: uuid::Uuid,
    ) -> Result<Option<wire_quotes::ApplicantActionProjection>> {
        self.quotes.load_applicant_action_projection(qid).await
    }

    pub async fn list_light(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
        now: TStamp,
    ) -> Result<Vec<LightQuote>> {
        let mut lights = self.quotes.list_light(filters, sort).await?;

        for light in lights.iter_mut() {
            if matches!(light.status, StatusDiscriminants::Offered) {
                let mut quote = self
                    .quotes
                    .load(light.id)
                    .await?
                    .ok_or(Error::InternalServer(String::from(
                        "light quote ID not found in quote",
                    )))?;
                let changed = quote.check_expire(now);
                if changed {
                    self.quotes
                        .update_status_if_offered(light.id, quote.status.clone())
                        .await?;
                    light.status = StatusDiscriminants::from(quote.status.clone());
                }
            }
        }
        Ok(lights)
    }

    #[cfg(test)]
    pub async fn offer(
        &self,
        qid: uuid::Uuid,
        discounted: btc::Amount,
        submitted: TStamp,
        ttl: Option<TStamp>,
    ) -> Result<(btc::Amount, TStamp)> {
        let mut quote = self._lookup(qid, submitted).await?;
        quote.require_credit_program()?;
        let Status::Pending { .. } = quote.status else {
            return Err(Error::InvalidQuoteStatus(
                qid,
                StatusDiscriminants::Pending,
                StatusDiscriminants::from(quote.status.clone()),
            ));
        };
        let expiration_date = calculate_expiration_from_maturity(quote.bill.maturity_date);
        let kid = self
            .wdc_client
            .get_keyset_with_expiration_date(expiration_date)
            .await?;
        let expiration = ttl.unwrap_or(calculate_default_expiration_date_for_quote(submitted));
        quote.offer(kid, expiration, discounted)?;
        self.quotes
            .update_status_if_pending(quote.id, quote.status)
            .await?;
        Ok((discounted, expiration))
    }

    pub async fn authorize_offer(
        &self,
        qid: uuid::Uuid,
        signed: wire_quotes::SignedCreditAuthorizationEnvelope,
        now: TStamp,
    ) -> Result<wire_quotes::CreditAuthorizationReceipt> {
        let verifier = &self.authorization_verifier;
        let mut quote = self
            .quotes
            .load(qid)
            .await?
            .ok_or_else(|| Error::ResourceNotFound(qid.to_string()))?;
        let existing = quote.authorization_receipt().cloned();
        let verified = if existing.is_some() {
            verifier.verify_replay(signed, &quote)?
        } else {
            verifier.verify(signed, &quote, now)?
        };
        if let Some(existing) = existing {
            return if existing.operation_id == verified.operation_id
                && existing.authorization_digest == verified.authorization_digest
            {
                Ok(existing)
            } else {
                Err(Error::CreditAuthorizationConflict)
            };
        }

        let expiration_date = calculate_expiration_from_maturity(quote.bill.maturity_date);
        let keyset_id = self
            .wdc_client
            .get_keyset_with_expiration_date(expiration_date)
            .await?;
        quote.offer(keyset_id, verified.expiration, verified.discounted)?;
        let receipt = wire_quotes::CreditAuthorizationReceipt {
            receipt_version: String::from("credit-authorization-receipt-v1"),
            operation_id: verified.operation_id,
            authorization_digest: verified.authorization_digest,
            case_id: verified.authorization.case_id,
            status: String::from("completed"),
            mint_id: verified.authorization.mint_id,
            bill_id: verified.authorization.bill_id,
            action: verified.authorization.action,
            effect_id: qid.to_string(),
            result_digest: offer_result_digest(qid, verified.discounted, verified.expiration),
            completed_at: now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            synthetic: true,
        };
        quote.authorization_receipt = Some(receipt);
        self.quotes.execute_authorization(quote).await
    }

    pub async fn set_failed_ebill_validation(&self, qid: uuid::Uuid) -> Result<()> {
        let mut quote = self
            .quotes
            .load(qid)
            .await?
            .ok_or(Error::ResourceNotFound(qid.to_string()))?;
        let Status::Accepted { .. } = quote.status else {
            return Err(Error::InvalidQuoteStatus(
                qid,
                StatusDiscriminants::Accepted,
                StatusDiscriminants::from(quote.status.clone()),
            ));
        };
        quote.set_failed_ebill_validation()?;
        self.quotes
            .update_status_if_accepted(quote.id, quote.status)
            .await?;
        Ok(())
    }

    pub async fn enable_minting_manual_override(&self, qid: uuid::Uuid) -> Result<()> {
        let mut quote = self
            .quotes
            .load(qid)
            .await?
            .ok_or(Error::ResourceNotFound(qid.to_string()))?;
        let Status::FailedEbillValidation {
            keyset_id,
            discounted,
            wallet_pubkey,
        } = quote.status
        else {
            return Err(Error::InvalidQuoteStatus(
                qid,
                StatusDiscriminants::FailedEbillValidation,
                StatusDiscriminants::from(quote.status.clone()),
            ));
        };
        let fees_amount = quote.bill.sum - discounted;
        let fees_amount = cashu::Amount::from(fees_amount.to_sat());
        quote.override_failed_ebill_validation(fees_amount)?;
        self.trigger_enable_minting_operations(
            qid,
            keyset_id,
            wallet_pubkey,
            fees_amount,
            discounted,
            quote.bill.id.clone(),
        )
        .await?;
        self.quotes
            .update_status_if_failedebillvalidation(quote.id, quote.status)
            .await?;
        Ok(())
    }

    pub async fn enable_minting(&self, qid: uuid::Uuid) -> Result<()> {
        let mut quote = self
            .quotes
            .load(qid)
            .await?
            .ok_or(Error::ResourceNotFound(qid.to_string()))?;
        let Status::Accepted {
            keyset_id,
            discounted,
            wallet_pubkey,
        } = quote.status
        else {
            return Err(Error::InvalidQuoteStatus(
                qid,
                StatusDiscriminants::Accepted,
                StatusDiscriminants::from(quote.status.clone()),
            ));
        };
        let fees_amount = quote.bill.sum - discounted;
        let fees_amount = cashu::Amount::from(fees_amount.to_sat());
        quote.start_minting(fees_amount)?;
        self.trigger_enable_minting_operations(
            qid,
            keyset_id,
            wallet_pubkey,
            fees_amount,
            discounted,
            quote.bill.id.clone(),
        )
        .await?;
        self.quotes
            .update_status_if_accepted(quote.id, quote.status)
            .await?;
        Ok(())
    }

    async fn trigger_enable_minting_operations(
        &self,
        qid: uuid::Uuid,
        keyset_id: cashu::Id,
        wallet_pubkey: cashu::PublicKey,
        fees_amount: cashu::Amount,
        discounted: btc::Amount,
        bill_id: BillId,
    ) -> Result<()> {
        let keys = self.wdc_client.get_keys(keyset_id).await?;
        let fees = mint_fees(self.wdc_client.as_ref(), fees_amount, keys).await?;
        let discounted_amount = cashu::Amount::from(discounted.to_sat());
        self.wdc_client
            .add_new_mint_operation(qid, keyset_id, wallet_pubkey, discounted_amount, bill_id)
            .await?;
        self.wdc_client.collect_fees(fees).await?;
        Ok(())
    }

    pub async fn check_if_endorsed_bill_is_valid(
        &self,
        bill_id: BillId,
        quote: Quote,
    ) -> Result<bool> {
        let res = self
            .wdc_client
            .validate_endorsed_bill_matches_shared_bill(bill_id, quote.bill.shared_bill_data)
            .await?;
        Ok(res)
    }

    pub async fn get_shared_ebill_history(
        &self,
        qid: uuid::Uuid,
    ) -> Result<Vec<wire_bill::BillHistoryBlock>> {
        let quote = self
            .quotes
            .load(qid)
            .await?
            .ok_or(Error::ResourceNotFound(qid.to_string()))?;
        let history_blocks = self
            .wdc_client
            .get_shared_ebill_history(quote.bill.id, quote.bill.shared_bill_data)
            .await?;
        Ok(history_blocks)
    }
}

pub fn calculate_default_expiration_date_for_quote(now: crate::TStamp) -> super::TStamp {
    now + chrono::Duration::days(2)
}

pub fn calculate_expiration_from_maturity(maturity_date: chrono::NaiveDate) -> chrono::NaiveDate {
    maturity_date + chrono::Duration::days(2)
}

fn same_reissue_bill(previous: &BillInfo, current: &BillInfo) -> bool {
    previous.id == current.id
        && previous.drawee == current.drawee
        && previous.drawer == current.drawer
        && previous.payee == current.payee
        && previous.endorsees == current.endorsees
        && previous.current_holder == current.current_holder
        && previous.sum == current.sum
        && previous.maturity_date == current.maturity_date
}

async fn mint_fees(
    keyscl: &dyn WdcClient,
    fees_amount: cashu::Amount,
    keys: cashu::KeySet,
) -> Result<Vec<cashu::Proof>> {
    let premint = cashu::PreMintSecrets::random(
        keys.id,
        fees_amount,
        &cashu::amount::SplitTarget::None,
        &bcr_wdc_utils::keys::to_fee_and_amounts(&keys),
    )
    .map_err(|e| Error::InternalServer(format!("mint_fees(): PreMintSecrets::random(): {e}")))?;
    let signatures = keyscl.sign(&premint.blinded_messages()).await?;
    let (rs, secrets) = premint
        .secrets
        .into_iter()
        .map(|secret| (secret.r, secret.secret))
        .unzip();
    let prfs = cashu::dhke::construct_proofs(signatures, rs, secrets, &keys.keys)
        .map_err(|e| Error::InternalServer(format!("mint_fees(): construct_proofs(): {e}")))?;
    Ok(prfs)
}

pub(crate) fn validate_basic_ebill_rules(bill: &BillInfo, today: chrono::NaiveDate) -> Result<()> {
    if bill.maturity_date < today {
        return Err(Error::InvalidInput(String::from(
            "maturity date must be >= today",
        )));
    }
    validate_basic_ebill_amount(bill)
}

pub(crate) fn validate_basic_ebill_amount(bill: &BillInfo) -> Result<()> {
    if bill.sum <= btc::Amount::ONE_SAT || bill.sum > bitcoin::Amount::MAX_MONEY {
        return Err(Error::InvalidInput(format!(
            "{} < bill_amount < {}",
            btc::Amount::ONE_SAT,
            btc::Amount::MAX_MONEY
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::MockRepository;
    use bcr_common::{core_tests, wire_tests};
    use bcr_ebill_core::protocol::blockchain::bill::participant::BillParticipant;
    use bcr_wdc_utils::{convert, keys::test_utils as keys_utils};
    use mockall::predicate::*;
    use rand::Rng;
    use std::str::FromStr;

    pub const TEST_URL: &str = "http://localhost:8000";

    fn generate_random_bill() -> BillInfo {
        let mut rng = rand::thread_rng();
        let holder =
            convert::billidentparticipant_wire2ebill(wire_tests::random_identity_public_data().1)
                .unwrap();
        BillInfo {
            id: core_tests::random_bill_id(),
            drawee: convert::billidentparticipant_wire2ebill(
                wire_tests::random_identity_public_data().1,
            )
            .unwrap(),
            drawer: convert::billidentparticipant_wire2ebill(
                wire_tests::random_identity_public_data().1,
            )
            .unwrap(),
            payee: BillParticipant::Ident(holder.clone()),
            current_holder: BillParticipant::Ident(holder),
            endorsees: Default::default(),
            sum: btc::Amount::from_sat(rng.gen_range(1000..100000)),
            maturity_date: (chrono::Utc::now() + chrono::Duration::days(rng.gen_range(10..30)))
                .date_naive(),
            file_urls: Vec::default(),
            shared_bill_data: String::default(),
        }
    }

    #[test]
    fn test_validate_basic_ebill_rules_maturity_date() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut bill = generate_random_bill();

        bill.maturity_date = today;
        assert!(validate_basic_ebill_rules(&bill, today).is_ok());

        bill.maturity_date = today - chrono::Duration::days(1);
        assert!(validate_basic_ebill_rules(&bill, today).is_err());
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_not_present() {
        let mut quotes = MockRepository::new();
        quotes.expect_search_by_bill().returning(|_, _| Ok(vec![]));
        quotes
            .expect_store_if_latest()
            .withf(|expected, quote| {
                expected.is_none()
                    && quote.credit_program().is_some_and(|binding| {
                    binding.version() == "test-credit-program-v1"
                        && binding.digest()
                            == "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                })
            })
            .returning(|_, quote| Ok(quote.id));
        let wdc_client = MockWdcClient::new();

        let rnd_bill = generate_random_bill();
        let service = Service {
            quotes: Box::new(quotes),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
            credit_program: crate::quotes::test_credit_program_binding(),
            authorization_verifier: crate::authorization::test_authorization_verifier(),
        };
        let test = service
            .enquire(rnd_bill, keys_utils::publics()[0], chrono::Utc::now())
            .await;
        assert!(test.is_ok());
    }

    #[tokio::test]
    async fn completed_authorization_replays_after_envelope_expiry() {
        let mut bill = generate_random_bill();
        bill.sum = btc::Amount::from_sat(8_000_000);
        bill.maturity_date = chrono::NaiveDate::from_ymd_opt(2027, 2, 6).unwrap();
        let quote = Quote::new(
            bill,
            keys_utils::publics()[0],
            TStamp::default(),
            crate::quotes::test_credit_program_binding(),
        );
        let qid = quote.id;
        let signed = crate::authorization::tests::signed_for(&quote);
        let db = crate::persistence::inmemory::QuotesIDMap::default();
        db.store(quote).await.unwrap();

        let mut wdc_client = MockWdcClient::new();
        wdc_client
            .expect_get_keyset_with_expiration_date()
            .times(1)
            .returning(|_| Ok(core_tests::generate_random_ecash_keyset().0.id));
        let service = Service {
            quotes: Box::new(db),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
            credit_program: crate::quotes::test_credit_program_binding(),
            authorization_verifier: crate::authorization::test_authorization_verifier(),
        };
        let issued = chrono::DateTime::parse_from_rfc3339("2026-08-10T12:05:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let first = service
            .authorize_offer(qid, signed.clone(), issued)
            .await
            .unwrap();

        let replayed = service
            .authorize_offer(qid, signed, issued + chrono::Duration::days(3))
            .await
            .unwrap();

        assert_eq!(replayed, first);
    }

    #[tokio::test]
    async fn governed_denial_replays_after_command_expiry() {
        let mut bill = generate_random_bill();
        bill.sum = btc::Amount::from_sat(8_000_000);
        bill.maturity_date = chrono::NaiveDate::from_ymd_opt(2027, 2, 6).unwrap();
        let quote = Quote::new(
            bill,
            keys_utils::publics()[0],
            TStamp::default(),
            crate::quotes::test_credit_program_binding(),
        );
        let qid = quote.id;
        let issued = chrono::DateTime::parse_from_rfc3339("2026-08-25T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let signed = crate::authorization::tests::signed_denial_for(
            &quote,
            issued,
            issued + chrono::Duration::hours(1),
        );
        let db = crate::persistence::inmemory::QuotesIDMap::default();
        db.store(quote).await.unwrap();
        let service = Service {
            quotes: Box::new(db),
            wdc_client: Box::new(MockWdcClient::new()),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
            credit_program: crate::quotes::test_credit_program_binding(),
            authorization_verifier: crate::authorization::test_authorization_verifier(),
        };

        let consumed_at = issued + chrono::Duration::microseconds(123);
        let first = service
            .deny_governed(signed.clone(), consumed_at)
            .await
            .unwrap();
        let stored = service.lookup(qid, consumed_at).await.unwrap();
        assert!(matches!(
            stored.status,
            Status::Denied { tstamp }
                if tstamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                    == first.completed_at
        ));
        let replayed = service
            .deny_governed(signed, issued + chrono::Duration::days(2))
            .await
            .unwrap();

        assert_eq!(replayed, first);
        assert_eq!(replayed.action, crate::authorization::QUOTE_DENIAL_ACTION);
        assert_eq!(replayed.effect_id, qid.to_string());
    }

    #[tokio::test]
    async fn unsigned_and_signed_governed_denial_have_one_allowed_outcome() {
        let mut bill = generate_random_bill();
        bill.sum = btc::Amount::from_sat(8_000_000);
        bill.maturity_date = chrono::NaiveDate::from_ymd_opt(2027, 2, 6).unwrap();
        let quote = Quote::new(
            bill,
            keys_utils::publics()[0],
            TStamp::default(),
            crate::quotes::test_credit_program_binding(),
        );
        let qid = quote.id;
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-25T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let signed = crate::authorization::tests::signed_denial_for(
            &quote,
            now,
            now + chrono::Duration::hours(1),
        );
        let db = crate::persistence::inmemory::QuotesIDMap::default();
        db.store(quote).await.unwrap();
        let service = Arc::new(Service {
            quotes: Box::new(db),
            wdc_client: Box::new(MockWdcClient::new()),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
            credit_program: crate::quotes::test_credit_program_binding(),
            authorization_verifier: crate::authorization::test_authorization_verifier(),
        });

        let (unsigned, governed) =
            tokio::join!(service.deny(qid, now), service.deny_governed(signed, now),);

        assert!(matches!(unsigned, Err(Error::CreditAuthorizationRequired)));
        assert!(governed.is_ok());
        let stored = service.lookup(qid, now).await.unwrap();
        assert!(matches!(stored.status, Status::Denied { .. }));
        assert_eq!(
            stored
                .authorization_receipt()
                .map(|receipt| receipt.action.as_str()),
            Some(crate::authorization::QUOTE_DENIAL_ACTION)
        );
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_pending() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let wallet_pubkey = keys_utils::publics()[0];
        let mut repo = MockRepository::new();
        let cloned = rnd_bill.clone();
        repo.expect_search_by_bill()
            .with(
                eq(rnd_bill.id.clone()),
                eq(rnd_bill.payee.node_id().clone()),
            )
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: Status::Pending { wallet_pubkey },
                    id,
                    bill: cloned.clone(),
                    submitted: chrono::Utc::now(),
                    credit_program: Some(crate::quotes::test_credit_program_binding()),
                    authorization_receipt: None,
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let wdc_client = MockWdcClient::new();

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
            credit_program: crate::quotes::test_credit_program_binding(),
            authorization_verifier: crate::authorization::test_authorization_verifier(),
        };
        let test_id = service
            .enquire(rnd_bill, wallet_pubkey, chrono::Utc::now())
            .await;
        assert!(test_id.is_ok());
        assert_eq!(id, test_id.unwrap());
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_denied() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let public_key = keys_utils::publics()[0];
        let cloned = rnd_bill.clone();
        let now = TStamp::from_timestamp(10000, 0).unwrap();
        let mut repo = MockRepository::new();
        repo.expect_search_by_bill()
            .with(
                eq(rnd_bill.id.clone()),
                eq(rnd_bill.payee.node_id().clone()),
            )
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: Status::Denied { tstamp: now },
                    id,
                    bill: cloned.clone(),
                    submitted: now,
                    credit_program: Some(crate::quotes::test_credit_program_binding()),
                    authorization_receipt: None,
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let wdc_client = MockWdcClient::new();

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
            credit_program: crate::quotes::test_credit_program_binding(),
            authorization_verifier: crate::authorization::test_authorization_verifier(),
        };
        let test_id = service.enquire(rnd_bill, public_key, now).await.unwrap();
        assert_eq!(id, test_id);
    }

    #[tokio::test]
    async fn legacy_denial_can_be_reenquired_after_user_decision_retention() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let public_key = keys_utils::publics()[0];
        let cloned = rnd_bill.clone();
        let denied_at = TStamp::from_timestamp(10_000, 0).unwrap();
        let mut repo = MockRepository::new();
        repo.expect_search_by_bill().returning(move |_, _| {
            Ok(vec![Quote {
                status: Status::Denied { tstamp: denied_at },
                id,
                bill: cloned.clone(),
                submitted: denied_at,
                credit_program: Some(crate::quotes::test_credit_program_binding()),
                authorization_receipt: None,
            }])
        });
        repo.expect_store_if_latest()
            .withf(move |expected, _| *expected == Some(id))
            .returning(|_, quote| Ok(quote.id));
        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(MockWdcClient::new()),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
            credit_program: crate::quotes::test_credit_program_binding(),
            authorization_verifier: crate::authorization::test_authorization_verifier(),
        };

        let submitted = denied_at + Service::USER_DECISION_RETENTION + chrono::Duration::seconds(1);
        let reissued = service
            .enquire(rnd_bill, public_key, submitted)
            .await
            .unwrap();

        assert_ne!(reissued, id);
    }

    #[tokio::test]
    async fn governed_denial_cannot_be_bypassed_by_legacy_reenquiry() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let public_key = keys_utils::publics()[0];
        let cloned = rnd_bill.clone();
        let denied_at = TStamp::from_timestamp(10_000, 0).unwrap();
        let mut repo = MockRepository::new();
        repo.expect_search_by_bill().returning(move |_, _| {
            Ok(vec![Quote {
                status: Status::Denied { tstamp: denied_at },
                id,
                bill: cloned.clone(),
                submitted: denied_at,
                credit_program: Some(crate::quotes::test_credit_program_binding()),
                authorization_receipt: Some(wire_quotes::CreditAuthorizationReceipt {
                    receipt_version: String::from("credit-authorization-receipt-v1"),
                    operation_id: format!("sha256:{}", "a".repeat(64)),
                    authorization_digest: format!("sha256:{}", "b".repeat(64)),
                    case_id: uuid::Uuid::from_u128(10).to_string(),
                    status: String::from("completed"),
                    mint_id: String::from("local-wildcat"),
                    bill_id: cloned.id.to_string(),
                    action: String::from(crate::authorization::QUOTE_DENIAL_ACTION),
                    effect_id: id.to_string(),
                    result_digest: format!("sha256:{}", "c".repeat(64)),
                    completed_at: denied_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    synthetic: true,
                }),
            }])
        });
        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(MockWdcClient::new()),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
            credit_program: crate::quotes::test_credit_program_binding(),
            authorization_verifier: crate::authorization::test_authorization_verifier(),
        };

        let submitted = denied_at + Service::USER_DECISION_RETENTION + chrono::Duration::seconds(1);
        let returned = service
            .enquire(rnd_bill, public_key, submitted)
            .await
            .unwrap();

        assert_eq!(returned, id);
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_offered() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let keyset_id = core_tests::generate_random_ecash_keyset().0.id;
        let wallet_pubkey = keys_utils::publics()[0];
        let now = TStamp::from_timestamp(10000, 0).unwrap();
        let cloned = rnd_bill.clone();
        let mut repo = MockRepository::new();
        repo.expect_search_by_bill()
            .with(
                eq(rnd_bill.id.clone()),
                eq(rnd_bill.payee.node_id().clone()),
            )
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: Status::Offered {
                        keyset_id,
                        ttl: now + chrono::Duration::days(1),
                        discounted: rnd_bill.sum,
                        wallet_pubkey,
                    },
                    id,
                    bill: cloned.clone(),
                    submitted: now,
                    credit_program: Some(crate::quotes::test_credit_program_binding()),
                    authorization_receipt: None,
                }])
            });
        repo.expect_store().returning(|_| Ok(()));
        let wdc_client = MockWdcClient::new();

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
            credit_program: crate::quotes::test_credit_program_binding(),
            authorization_verifier: crate::authorization::test_authorization_verifier(),
        };
        let test_id = service.enquire(rnd_bill, wallet_pubkey, now).await.unwrap();
        assert_eq!(id, test_id);
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_offered_but_expired() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let cloned = rnd_bill.clone();
        let keyset_id = core_tests::generate_random_ecash_keyset().0.id;
        let wallet_pubkey = keys_utils::publics()[0];
        let mut repo = MockRepository::new();
        let now = TStamp::from_timestamp(10000, 0).unwrap();
        repo.expect_search_by_bill()
            .with(
                eq(rnd_bill.id.clone()),
                eq(rnd_bill.payee.node_id().clone()),
            )
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: Status::Offered {
                        keyset_id,
                        ttl: now,
                        discounted: rnd_bill.sum,
                        wallet_pubkey,
                    },
                    id,
                    bill: cloned.clone(),
                    submitted: now,
                    credit_program: Some(crate::quotes::test_credit_program_binding()),
                    authorization_receipt: None,
                }])
            });
        repo.expect_update_status_if_offered()
            .returning(|_, _| Ok(()));
        let wdc_client = MockWdcClient::new();

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
            credit_program: crate::quotes::test_credit_program_binding(),
            authorization_verifier: crate::authorization::test_authorization_verifier(),
        };
        let test_id = service
            .enquire(rnd_bill, wallet_pubkey, now + chrono::Duration::seconds(1))
            .await
            .unwrap();
        assert_eq!(id, test_id);
    }

    #[tokio::test]
    async fn test_new_quote_request_quote_offered_expired_retention_passed() {
        let id = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let cloned = rnd_bill.clone();
        let keyset_id = core_tests::generate_random_ecash_keyset().0.id;
        let wallet_pubkey = keys_utils::publics()[0];
        let mut repo = MockRepository::new();
        let now = TStamp::from_timestamp(10000, 0).unwrap();
        repo.expect_search_by_bill()
            .with(
                eq(rnd_bill.id.clone()),
                eq(rnd_bill.payee.node_id().clone()),
            )
            .returning(move |_, _| {
                Ok(vec![Quote {
                    status: Status::Offered {
                        keyset_id,
                        ttl: now,
                        discounted: rnd_bill.sum,
                        wallet_pubkey,
                    },
                    id,
                    bill: cloned.clone(),
                    submitted: now,
                    credit_program: Some(crate::quotes::test_credit_program_binding()),
                    authorization_receipt: None,
                }])
            });
        repo.expect_update_status_if_offered()
            .returning(|_, _| Ok(()));
        repo.expect_store_if_latest()
            .withf(move |expected, _| *expected == Some(id))
            .returning(|_, quote| Ok(quote.id));
        let wdc_client = MockWdcClient::new();

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
            credit_program: crate::quotes::test_credit_program_binding(),
            authorization_verifier: crate::authorization::test_authorization_verifier(),
        };
        let submitted = now + Service::USER_DECISION_RETENTION + chrono::Duration::seconds(1);
        let test_id = service.enquire(rnd_bill, wallet_pubkey, submitted).await;
        assert!(test_id.is_ok());
        assert_ne!(id, test_id.unwrap());
    }

    #[tokio::test]
    async fn test_enable_minting_manual_override_quote_not_found() {
        let qid = Uuid::new_v4();
        let mut repo = MockRepository::new();
        repo.expect_load()
            .with(eq(qid))
            .times(1)
            .returning(|_| Ok(None));
        let wdc_client = MockWdcClient::new();

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
            credit_program: crate::quotes::test_credit_program_binding(),
            authorization_verifier: crate::authorization::test_authorization_verifier(),
        };
        let res = service.enable_minting_manual_override(qid).await;
        assert!(matches!(
            res,
            Err(Error::ResourceNotFound(id)) if id == qid.to_string()
        ));
    }

    #[tokio::test]
    async fn test_enable_minting_manual_override_invalid_status() {
        let qid = Uuid::new_v4();
        let rnd_bill = generate_random_bill();
        let wallet_pubkey = keys_utils::publics()[0];
        let mut repo = MockRepository::new();
        repo.expect_load()
            .with(eq(qid))
            .times(1)
            .returning(move |_| {
                Ok(Some(Quote {
                    id: qid,
                    status: Status::Pending { wallet_pubkey },
                    bill: rnd_bill.clone(),
                    submitted: chrono::Utc::now(),
                    credit_program: Some(crate::quotes::test_credit_program_binding()),
                    authorization_receipt: None,
                }))
            });

        let wdc_client = MockWdcClient::new();

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
            credit_program: crate::quotes::test_credit_program_binding(),
            authorization_verifier: crate::authorization::test_authorization_verifier(),
        };
        let res = service.enable_minting_manual_override(qid).await;
        assert!(matches!(
            res,
            Err(Error::InvalidQuoteStatus(
                id,
                StatusDiscriminants::FailedEbillValidation,
                StatusDiscriminants::Pending,
            )) if id == qid
        ));
    }

    #[tokio::test]
    async fn test_enable_minting_manual_override_success() {
        let qid = Uuid::new_v4();
        let mut quote = Quote::new(
            generate_random_bill(),
            keys_utils::publics()[0],
            chrono::Utc::now(),
            crate::quotes::test_credit_program_binding(),
        );
        quote.id = qid;
        let (_keyset_info, signing_keyset) = core_tests::generate_random_ecash_keyset();
        let keyset = cashu::KeySet {
            id: signing_keyset.id,
            unit: signing_keyset.unit.clone(),
            active: None,
            keys: signing_keyset.keys.clone().into(),
            input_fee_ppk: signing_keyset.input_fee_ppk,
            final_expiry: signing_keyset.final_expiry,
        };
        let keyset_id = keyset.id;
        let wallet_pubkey = keys_utils::publics()[0];
        let fee = cashu::Amount::from(10);
        let discounted = quote.bill.sum - btc::Amount::from_sat(10);
        let bill_id = quote.bill.id.clone();
        quote.status = Status::FailedEbillValidation {
            keyset_id,
            discounted,
            wallet_pubkey,
        };

        let mut repo = MockRepository::new();
        repo.expect_load()
            .with(eq(qid))
            .times(1)
            .returning(move |_| {
                Ok(Some(Quote {
                    id: qid,
                    status: Status::FailedEbillValidation {
                        keyset_id,
                        discounted,
                        wallet_pubkey,
                    },
                    bill: quote.bill.clone(),
                    submitted: chrono::Utc::now(),
                    credit_program: Some(crate::quotes::test_credit_program_binding()),
                    authorization_receipt: None,
                }))
            });
        repo.expect_update_status_if_failedebillvalidation()
            .withf(move |id, status| {
                *id == qid
                    && matches!(
                        status,
                        Status::MintingEnabled {
                            keyset_id: actual_keyset_id,
                            discounted: actual_discounted,
                            wallet_pubkey: actual_wallet_pubkey,
                            fee: actual_fee
                        } if *actual_keyset_id == keyset_id
                            && *actual_discounted == discounted
                            && *actual_wallet_pubkey == wallet_pubkey
                            && *actual_fee == fee
                    )
            })
            .times(1)
            .returning(|_, _| Ok(()));

        let mut wdc_client = MockWdcClient::new();
        wdc_client
            .expect_get_keys()
            .with(eq(keyset_id))
            .times(1)
            .returning(move |_| Ok(keyset.clone()));

        let signing_keyset = signing_keyset.clone();
        wdc_client.expect_sign().times(1).returning(move |msgs| {
            let amounts = msgs.iter().map(|msg| msg.amount).collect::<Vec<_>>();
            Ok(core_tests::generate_ecash_signatures(
                &signing_keyset,
                &amounts,
            ))
        });
        wdc_client
            .expect_add_new_mint_operation()
            .withf(
                move |actual_qid,
                      actual_keyset_id,
                      actual_wallet_pubkey,
                      target,
                      actual_bill_id| {
                    *actual_qid == qid
                        && *actual_keyset_id == keyset_id
                        && *actual_wallet_pubkey == wallet_pubkey
                        && *target == cashu::Amount::from(discounted.to_sat())
                        && *actual_bill_id == bill_id
                },
            )
            .times(1)
            .returning(|_, _, _, _, _| Ok(()));
        wdc_client
            .expect_collect_fees()
            .times(1)
            .returning(|_| Ok(()));

        let service = Service {
            quotes: Box::new(repo),
            wdc_client: Box::new(wdc_client),
            mint_url: cashu::MintUrl::from_str(TEST_URL).unwrap(),
            credit_program: crate::quotes::test_credit_program_binding(),
            authorization_verifier: crate::authorization::test_authorization_verifier(),
        };
        let res = service.enable_minting_manual_override(qid).await;
        assert!(res.is_ok());
    }
}
