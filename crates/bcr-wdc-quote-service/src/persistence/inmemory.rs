// ----- standard library imports
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_common::core::{BillId, NodeId};
use bcr_common::wire::quotes::SignedCreditQuoteReissuePermit;
use strum::IntoDiscriminant;
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::{
    authorization::same_quote_reissue_authority,
    error::{Error, Result},
    persistence::{
        same_executed_quote, same_governed_denial_authority, same_pending_quote_request,
        GovernedDenialInput, Repository,
    },
    quotes,
    service::{ListFilters, SortOrder},
};

#[allow(dead_code)]
#[derive(Default, Clone, Debug)]
pub struct QuotesIDMap {
    quotes: Arc<RwLock<HashMap<Uuid, quotes::Quote>>>,
    quote_reissues: Arc<RwLock<HashMap<Uuid, QuoteReissueRecord>>>,
    governed_denials:
        Arc<RwLock<HashMap<Uuid, bcr_common::wire::quotes::CreditAuthorizationReceipt>>>,
    applicant_actions:
        Arc<RwLock<HashMap<Uuid, crate::persistence::ApplicantActionProjectionState>>>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct QuoteReissueRecord {
    signed: SignedCreditQuoteReissuePermit,
    quote_id: Uuid,
    minting_pubkey: bcr_common::cashu::PublicKey,
    #[allow(dead_code)]
    consumed_at: crate::TStamp,
}

#[async_trait]
impl Repository for QuotesIDMap {
    async fn search_by_bill(&self, bill: &BillId, endorser: &NodeId) -> Result<Vec<quotes::Quote>> {
        Ok(self
            .quotes
            .read()
            .unwrap()
            .iter()
            .filter(|(_, quote)| {
                let holder = &quote
                    .bill
                    .endorsees
                    .last()
                    .unwrap_or(&quote.bill.payee)
                    .node_id();
                quote.bill.id == *bill && holder == endorser
            })
            .map(|x| x.1.clone())
            .collect())
    }

    async fn store(&self, quote: quotes::Quote) -> Result<()> {
        if quote.credit_program().is_none() {
            return Err(Error::CreditProgramNotBound(quote.id));
        }
        self.quotes.write().unwrap().insert(quote.id, quote);
        Ok(())
    }

    async fn store_if_latest(
        &self,
        expected_latest: Option<Uuid>,
        quote: quotes::Quote,
    ) -> Result<Uuid> {
        if quote.credit_program().is_none() {
            return Err(Error::CreditProgramNotBound(quote.id));
        }
        let holder = quote.bill.current_holder.node_id();
        let mut stored = self.quotes.write().unwrap();
        let latest = stored
            .values()
            .filter(|candidate| {
                candidate.bill.id == quote.bill.id
                    && candidate.bill.current_holder.node_id() == holder
            })
            .max_by_key(|candidate| (candidate.submitted, candidate.id));
        if latest.map(|candidate| candidate.id) != expected_latest {
            return latest
                .filter(|candidate| same_pending_quote_request(candidate, &quote))
                .map(|candidate| candidate.id)
                .ok_or(Error::CreditQuoteReissueConflict);
        }
        let id = quote.id;
        if stored.insert(id, quote).is_some() {
            return Err(Error::CreditQuoteReissueConflict);
        }
        Ok(id)
    }

    async fn execute_quote_reissue(
        &self,
        signed: SignedCreditQuoteReissuePermit,
        quote: quotes::Quote,
        consumed_at: crate::TStamp,
    ) -> Result<Uuid> {
        let previous_quote_id = signed.permit.previous_mint_quote_id;
        if quote.id != signed.permit.reissued_mint_quote_id {
            return Err(Error::CreditQuoteReissueConflict);
        }
        let mut records = self.quote_reissues.write().unwrap();
        let mut stored_quotes = self.quotes.write().unwrap();
        if let Some(record) = records.get(&previous_quote_id) {
            let same_permit = record.signed.permit_digest == signed.permit_digest
                || same_quote_reissue_authority(&record.signed.permit, &signed.permit);
            return match stored_quotes.get(&record.quote_id) {
                Some(existing)
                    if same_permit
                        && record.quote_id == quote.id
                        && same_executed_quote(existing, &quote, record.minting_pubkey) =>
                {
                    Ok(record.quote_id)
                }
                _ => Err(Error::CreditQuoteReissueConflict),
            };
        }
        crate::service::validate_basic_ebill_rules(&quote.bill, consumed_at.date_naive())?;
        let expires_at = chrono::DateTime::parse_from_rfc3339(&signed.permit.expires_at)
            .map_err(|_| Error::CreditQuoteReissueInvalid)?
            .with_timezone(&chrono::Utc);
        if expires_at <= consumed_at {
            return Err(Error::CreditQuoteReissueInvalid);
        }
        let minting_pubkey = match &quote.status {
            quotes::Status::Pending { wallet_pubkey } => *wallet_pubkey,
            _ => return Err(Error::CreditQuoteReissueConflict),
        };
        if stored_quotes.contains_key(&quote.id) {
            return Err(Error::CreditQuoteReissueConflict);
        }
        let holder = quote.bill.current_holder.node_id();
        let previous = stored_quotes.get(&previous_quote_id);
        if !matches!(previous, Some(candidate) if matches!(candidate.status, quotes::Status::Denied { .. }))
        {
            return Err(Error::CreditQuoteReissueConflict);
        }
        let previous_submitted = previous.expect("matched a denied quote").submitted;
        if stored_quotes.values().any(|candidate| {
            candidate.id != previous_quote_id
                && candidate.bill.id == quote.bill.id
                && candidate.bill.current_holder.node_id() == holder
                && candidate.submitted >= previous_submitted
        }) {
            return Err(Error::CreditQuoteReissueConflict);
        }
        let id = quote.id;
        stored_quotes.insert(id, quote);
        records.insert(
            previous_quote_id,
            QuoteReissueRecord {
                signed,
                quote_id: id,
                minting_pubkey,
                consumed_at,
            },
        );
        Ok(id)
    }
    async fn load(&self, id: uuid::Uuid) -> Result<Option<quotes::Quote>> {
        Ok(self.quotes.read().unwrap().get(&id).cloned())
    }

    async fn load_applicant_action_projection(
        &self,
        id: uuid::Uuid,
    ) -> Result<Option<bcr_common::wire::quotes::ApplicantActionProjection>> {
        if !self.quotes.read().unwrap().contains_key(&id) {
            return Err(Error::ResourceNotFound(id.to_string()));
        }
        Ok(self
            .applicant_actions
            .read()
            .unwrap()
            .get(&id)
            .and_then(|state| state.projection.clone()))
    }

    async fn apply_applicant_action_projection(
        &self,
        mutation: crate::persistence::ApplicantActionProjectionMutation,
    ) -> Result<bcr_common::wire::quotes::CreditApplicantActionReceipt> {
        let mut states = self.applicant_actions.write().unwrap();
        if let Some(current) = states.get(&mutation.quote_id) {
            if current.last_operation_id == mutation.operation_id {
                return Ok(current.receipt.clone());
            }
            if current.last_revision_digest != mutation.expected_revision_digest {
                return Err(Error::ApplicantActionProjectionConflict);
            }
        } else if mutation.expected_revision_digest.is_some() {
            return Err(Error::ApplicantActionProjectionConflict);
        }
        let stored_quotes = self.quotes.read().unwrap();
        let quote = stored_quotes
            .get(&mutation.quote_id)
            .ok_or_else(|| Error::ResourceNotFound(mutation.quote_id.to_string()))?;
        if !matches!(quote.status, quotes::Status::Pending { .. }) {
            return Err(Error::ApplicantActionProjectionConflict);
        }
        if mutation.expires_at <= mutation.applied_at {
            return Err(Error::ApplicantActionProjectionInvalid);
        }
        if mutation
            .projection
            .as_ref()
            .is_some_and(|projection| projection.revision_digest != mutation.revision_digest)
        {
            return Err(Error::ApplicantActionProjectionInvalid);
        }
        if mutation.receipt.schema_version != "credit-applicant-action-receipt-v1"
            || mutation.receipt.operation_id != mutation.operation_id
            || mutation.receipt.mint_quote_id != mutation.quote_id
            || mutation.receipt.revision_digest != mutation.revision_digest
            || mutation.receipt.expected_revision_digest != mutation.expected_revision_digest
            || mutation.receipt.action != crate::authorization::APPLICANT_ACTION_COMMAND_ACTION
            || mutation.receipt.status != "completed"
            || matches!(
                (&mutation.receipt.applicant_action, &mutation.projection),
                (
                    bcr_common::wire::quotes::CreditApplicantAction::ClarificationRequired,
                    None
                ) | (
                    bcr_common::wire::quotes::CreditApplicantAction::None,
                    Some(_)
                )
            )
        {
            return Err(Error::ApplicantActionProjectionInvalid);
        }
        match (states.get(&mutation.quote_id), &mutation.projection) {
            (Some(current), Some(projection))
                if current.last_revision_digest.as_ref() == Some(&projection.revision_digest) =>
            {
                return Err(Error::ApplicantActionProjectionConflict);
            }
            (Some(current), None) if current.projection.is_none() => {
                return Err(Error::ApplicantActionProjectionConflict);
            }
            (None, None) => return Err(Error::ApplicantActionProjectionConflict),
            _ => {}
        }
        if mutation.expected_revision_digest.as_ref() == Some(&mutation.revision_digest) {
            return Err(Error::ApplicantActionProjectionConflict);
        }
        let last_revision_digest = Some(mutation.revision_digest);
        let projection = mutation.projection;
        states.insert(
            mutation.quote_id,
            crate::persistence::ApplicantActionProjectionState {
                quote_id: mutation.quote_id,
                projection: projection.clone(),
                last_revision_digest,
                last_operation_id: mutation.operation_id,
                last_command_digest: mutation.command_digest,
                receipt: mutation.receipt.clone(),
            },
        );
        drop(stored_quotes);
        Ok(mutation.receipt)
    }

    async fn update_status_if_pending(&self, qid: uuid::Uuid, new: quotes::Status) -> Result<()> {
        let mut m = self.quotes.write().unwrap();
        let result = m.get_mut(&qid);
        if let Some(old) = result {
            if matches!(old.status, quotes::Status::Pending { .. }) {
                old.status = new;
                return Ok(());
            }
        }
        Err(Error::QuotesRepository(anyhow!(
            "quote {qid} not found or not pending"
        )))
    }

    async fn execute_authorization(
        &self,
        quote: quotes::Quote,
    ) -> Result<bcr_common::wire::quotes::CreditAuthorizationReceipt> {
        let receipt = quote
            .authorization_receipt
            .clone()
            .ok_or(Error::CreditAuthorizationInvalid)?;
        let mut quotes = self.quotes.write().unwrap();
        let stored = quotes
            .get_mut(&quote.id)
            .ok_or_else(|| Error::ResourceNotFound(quote.id.to_string()))?;
        if let Some(existing) = &stored.authorization_receipt {
            return if existing.operation_id == receipt.operation_id
                && existing.authorization_digest == receipt.authorization_digest
            {
                Ok(existing.clone())
            } else {
                Err(Error::CreditAuthorizationConflict)
            };
        }
        if !matches!(stored.status, quotes::Status::Pending { .. }) {
            return Err(Error::CreditAuthorizationConflict);
        }
        stored.status = quote.status;
        stored.authorization_receipt = Some(receipt.clone());
        Ok(receipt)
    }

    async fn execute_governed_denial(
        &self,
        input: GovernedDenialInput,
    ) -> Result<bcr_common::wire::quotes::CreditAuthorizationReceipt> {
        let mut denials = self.governed_denials.write().unwrap();
        let mut quotes = self.quotes.write().unwrap();
        let stored = quotes
            .get_mut(&input.quote_id)
            .ok_or_else(|| Error::ResourceNotFound(input.quote_id.to_string()))?;
        if let Some(existing) = denials.get(&input.quote_id) {
            return if stored.authorization_receipt.as_ref() == Some(existing)
                && matches!(stored.status, quotes::Status::Denied { .. })
                && same_governed_denial_authority(existing, &input.receipt)
            {
                Ok(existing.clone())
            } else {
                Err(Error::CreditQuoteDenialConflict)
            };
        }
        if stored.authorization_receipt.is_some() {
            return Err(Error::CreditQuoteDenialConflict);
        }
        if input.expires_at <= input.denied_at {
            return Err(Error::CreditQuoteDenialInvalid);
        }
        if !matches!(stored.status, quotes::Status::Pending { .. })
            || stored.credit_program().is_none()
            || input.receipt.effect_id != input.quote_id.to_string()
            || input.receipt.bill_id != stored.bill.id.to_string()
            || input.receipt.action != crate::authorization::QUOTE_DENIAL_ACTION
            || input.receipt.status != "completed"
        {
            return Err(Error::CreditQuoteDenialConflict);
        }
        stored.status = quotes::Status::Denied {
            tstamp: input.denied_at,
        };
        stored.authorization_receipt = Some(input.receipt.clone());
        denials.insert(input.quote_id, input.receipt.clone());
        Ok(input.receipt)
    }

    async fn update_status_if_offered(&self, qid: uuid::Uuid, new: quotes::Status) -> Result<()> {
        let mut m = self.quotes.write().unwrap();
        let result = m.get_mut(&qid);
        if let Some(old) = result {
            if matches!(old.status, quotes::Status::Offered { .. }) {
                old.status = new;
                return Ok(());
            }
        }
        Err(Error::QuotesRepository(anyhow!(
            "quote {qid} not found or not offered"
        )))
    }

    async fn update_status_if_accepted(&self, qid: uuid::Uuid, new: quotes::Status) -> Result<()> {
        let mut m = self.quotes.write().unwrap();
        let result = m.get_mut(&qid);
        if let Some(old) = result {
            if matches!(old.status, quotes::Status::Accepted { .. }) {
                old.status = new;
                return Ok(());
            }
        }
        Err(Error::QuotesRepository(anyhow!(
            "quote {qid} not found or not accepted"
        )))
    }

    async fn update_status_if_failedebillvalidation(
        &self,
        qid: uuid::Uuid,
        new: quotes::Status,
    ) -> Result<()> {
        let mut m = self.quotes.write().unwrap();
        let result = m.get_mut(&qid);
        if let Some(old) = result {
            if matches!(old.status, quotes::Status::FailedEbillValidation { .. }) {
                old.status = new;
                return Ok(());
            }
        }
        Err(Error::QuotesRepository(anyhow!(
            "quote {qid} not found or not failedebillvalidation"
        )))
    }

    async fn list_light(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
    ) -> Result<Vec<quotes::LightQuote>> {
        let mut a: Vec<quotes::Quote> = self
            .quotes
            .read()
            .unwrap()
            .iter()
            .filter(|(_, quote)| {
                let ListFilters {
                    bill_maturity_date_from,
                    bill_maturity_date_to,
                    status,
                    bill_id,
                    bill_drawee_id,
                    bill_drawer_id,
                    bill_payer_id,
                    bill_holder_id,
                } = &filters;
                if let Some(bill_maturity_date_from) = bill_maturity_date_from {
                    if quote.bill.maturity_date < *bill_maturity_date_from {
                        return false;
                    }
                }
                if let Some(bill_maturity_date_to) = bill_maturity_date_to {
                    if quote.bill.maturity_date > *bill_maturity_date_to {
                        return false;
                    }
                }
                if let Some(status) = status {
                    if quote.status.discriminant() != *status {
                        return false;
                    }
                }
                if let Some(bill_id) = bill_id {
                    if quote.bill.id != *bill_id {
                        return false;
                    }
                }
                if let Some(bill_drawee_id) = bill_drawee_id {
                    if quote.bill.drawee.node_id != *bill_drawee_id {
                        return false;
                    }
                }
                if let Some(bill_drawer_id) = bill_drawer_id {
                    if quote.bill.drawer.node_id != *bill_drawer_id {
                        return false;
                    }
                }
                if let Some(bill_payer_id) = bill_payer_id {
                    if quote.bill.payee.node_id() != *bill_payer_id {
                        return false;
                    }
                }
                if let Some(bill_holder_id) = bill_holder_id {
                    let holder_id = &quote
                        .bill
                        .endorsees
                        .last()
                        .unwrap_or(&quote.bill.payee)
                        .node_id();
                    if *holder_id != *bill_holder_id {
                        return false;
                    }
                }
                true
            })
            .map(|(_, quote)| quote.clone())
            .collect();
        if let Some(sort) = sort {
            a.sort_by(|q1, q2| match sort {
                SortOrder::BillMaturityDateAsc => q1.bill.maturity_date.cmp(&q2.bill.maturity_date),
                SortOrder::BillMaturityDateDesc => {
                    q2.bill.maturity_date.cmp(&q1.bill.maturity_date)
                }
                SortOrder::SubmittedAsc => q1.submitted.cmp(&q2.submitted),
                SortOrder::SubmittedDesc => q2.submitted.cmp(&q1.submitted),
            });
        }
        let b = a
            .into_iter()
            .map(|quote| quotes::LightQuote {
                id: quote.id,
                status: quote.status.discriminant(),
                sum: quote.bill.sum,
                maturity_date: quote.bill.maturity_date,
            })
            .collect();
        Ok(b)
    }
}
