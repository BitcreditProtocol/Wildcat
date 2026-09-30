// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu,
    client::{
        admin::clowder::Client as ClowderRestClient, clowder::ClowderNatsClient,
        core::Client as CoreClient, ebill::Client as EbillClient,
    },
    clowder::taproot,
    core::{self, BillId},
    ecash,
    wire::{bill as wire_bill, clowder as wire_clowder},
};
use bitcoin::hashes::Hash;
use uuid::Uuid;
// ----- local imports
use crate::{
    ebill,
    error::{Error, Result},
    TStamp,
};

// ----- end imports

pub struct ClwdrCl {
    pub rest: Arc<ClowderRestClient>,
    pub nats: Arc<ClowderNatsClient>,
}

#[async_trait]
impl ebill::ClowderClient for ClwdrCl {
    async fn register_ebill(&self, bid: BillId, amount: cashu::Amount) -> Result<()> {
        let request = wire_clowder::RegisterEbillRequest {
            bill_id: bid,
            amount,
        };
        let response = wire_clowder::RegisterEbillResponse {};
        let _resp = self.nats.register_ebill(request, response).await?;
        Ok(())
    }

    async fn minting_ebill(
        &self,
        keyset_id: cashu::Id,
        quote_id: Uuid,
        amount: cashu::Amount,
        bill_id: core::BillId,
        signatures: Vec<cashu::BlindSignature>,
    ) -> Result<Vec<cashu::BlindSignature>> {
        let request = wire_clowder::MintEbillRequest {
            keyset_id,
            amount,
            bill_id,
            quote_id,
        };
        let response = wire_clowder::MintEbillResponse { signatures };
        let res = self.nats.mint_bill(request, response).await?;
        Ok(res.signatures)
    }

    async fn request_to_pay_ebill(
        &self,
        bid: BillId,
        payment_address: bitcoin::Address,
        block_id: u64,
        previous_block_hash: bitcoin::hashes::sha256::Hash,
        amount: bitcoin::Amount,
    ) -> Result<()> {
        let req = wire_clowder::RequestToPayEbillRequest {
            bill_id: bid,
            payment_address: payment_address.into_unchecked(),
            block_id,
            previous_block_hash,
            amount,
        };
        let resp = wire_clowder::RequestToPayEbillResponse {};
        let _resp = self.nats.request_to_pay_bill(req, resp).await?;
        Ok(())
    }

    async fn request_onchain_ebill_address(
        &self,
        bid: BillId,
        block_id: u64,
        previous_block_hash: bitcoin::hashes::sha256::Hash,
    ) -> Result<bitcoin::Address> {
        let info = self.rest.get_info().await?;
        let network = info.network;
        let frost_agg_key = info.multisig_agg_xonly;
        let derived_address = taproot::derive_ebill_mint_req_to_pay_address(
            &frost_agg_key,
            &bid,
            block_id,
            previous_block_hash.as_byte_array(),
            network,
        )
        .map_err(|e| Error::Internal(e.to_string()))?;
        Ok(derived_address)
    }
}

pub struct WildcatCl {
    pub core: Arc<CoreClient>,
    pub ebill: Box<EbillClient>,
}

/// The history projection is produced from the eBill service's locally validated
/// chain. Ordinary endorsement to the Mint is not an authorized Mint transfer.
fn history_confirms_mint_holder(
    history: &[wire_bill::BillHistoryBlock],
    mint: &core::NodeId,
) -> bool {
    if history.iter().any(|block| {
        !matches!(
            block.block_type.as_str(),
            "Issue"
                | "Endorse"
                | "Mint"
                | "Sell"
                | "Recourse"
                | "RequestToAccept"
                | "Accept"
                | "RequestToPay"
                | "OfferToSell"
                | "RejectToAccept"
                | "RejectToPay"
                | "RejectToPayRecourse"
                | "RejectToBuy"
                | "RequestRecourse"
        )
    }) {
        return false;
    }
    history
        .iter()
        .filter(|block| {
            matches!(
                block.block_type.as_str(),
                "Issue" | "Endorse" | "Mint" | "Sell" | "Recourse"
            )
        })
        .max_by_key(|block| block.block_id)
        .is_some_and(|block| {
            block.block_type == "Mint"
                && block
                    .pay_to_the_order_of
                    .as_ref()
                    .is_some_and(|holder| holder.node_id() == *mint)
        })
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    #[test]
    fn ordinary_return_to_mint_cannot_reauthorize_an_old_mint_operation() {
        let mint = bcr_common::core_tests::random_node_id();
        let other = bcr_common::core_tests::random_node_id();
        let block = |id, kind: &str, owner: Option<core::NodeId>| wire_bill::BillHistoryBlock {
            block_id: id,
            block_type: kind.into(),
            pay_to_the_order_of: owner.map(|node_id| {
                wire_bill::BillParticipant::Anon(wire_bill::BillAnonParticipant {
                    node_id,
                    nostr_relays: vec![],
                })
            }),
            payment_data: None,
            request_deadline: None,
            signed: wire_bill::SignedBy {
                data: wire_bill::BillParticipant::Anon(wire_bill::BillAnonParticipant {
                    node_id: mint.clone(),
                    nostr_relays: vec![],
                }),
                signatory: None,
            },
            signing_timestamp: id,
            signing_address: None,
        };
        assert!(!history_confirms_mint_holder(&[], &mint));
        assert!(!history_confirms_mint_holder(
            &[block(1, "Issue", None)],
            &mint
        ));
        let minted = block(2, "Mint", Some(mint.clone()));
        assert!(history_confirms_mint_holder(
            &[minted.clone(), block(3, "RequestToPay", None)],
            &mint
        ));
        assert!(!history_confirms_mint_holder(
            &[minted.clone(), block(3, "Endorse", Some(other.clone()))],
            &mint
        ));
        assert!(!history_confirms_mint_holder(
            &[
                minted.clone(),
                block(3, "Endorse", Some(other)),
                block(4, "Endorse", Some(mint.clone()))
            ],
            &mint
        ));
        assert!(!history_confirms_mint_holder(
            &[minted.clone(), block(3, "Recourse", Some(mint.clone()))],
            &mint
        ));
        assert!(!history_confirms_mint_holder(
            &[minted, block(3, "UnknownTransfer", Some(mint.clone()))],
            &mint
        ));
    }
}

#[async_trait]
impl ebill::WildcatClient for WildcatCl {
    async fn bill_is_held_by_mint(&self, bid: BillId) -> Result<bool> {
        let identity = self.ebill.get_identity().await?;
        let bill = self.ebill.get_bill(&bid).await?;
        let history = self.ebill.get_bill_history(bid.clone()).await?;
        let holder = bill
            .participants
            .endorsee
            .as_ref()
            .unwrap_or(&bill.participants.payee);
        Ok(bill.id == bid
            && holder.node_id() == identity.node_id
            && bill.status.acceptance.accepted
            && !bill.status.payment.paid
            && history_confirms_mint_holder(&history, &identity.node_id))
    }

    async fn info(&self, kid: cashu::Id) -> Result<ecash::KeySetInfo> {
        let kinfo = self.core.keyset_info(kid).await?;
        Ok(kinfo)
    }

    async fn sign(&self, blinds: &[cashu::BlindedMessage]) -> Result<Vec<cashu::BlindSignature>> {
        let c_blinds: Vec<_> = blinds.iter().cloned().map(From::from).collect();
        let res = self.core.sign(&c_blinds).await?;
        Ok(res.into_iter().map(From::from).collect())
    }

    async fn burn(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        let c_proofs = proofs.into_iter().map(From::from).collect();
        self.core.burn(c_proofs).await?;
        Ok(())
    }

    async fn recover(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        let c_proofs = proofs.into_iter().map(From::from).collect();
        self.core.recover(c_proofs).await?;
        Ok(())
    }

    async fn prepare_request_to_pay(
        &self,
        bid: core::BillId,
    ) -> Result<(u64, bitcoin::hashes::sha256::Hash)> {
        let request = wire_bill::PrepareRequestToPayBitcreditBillPayload { bill_id: bid };
        let resp: wire_bill::PrepareRequestToPayBitcreditBillResponse =
            self.ebill.prepare_request_to_pay_bill(&request).await?;

        Ok((resp.block_id, resp.previous_block_hash))
    }

    async fn request_to_pay(
        &self,
        bill_id: core::BillId,
        deadline: TStamp,
        payment_address: bitcoin::Address,
    ) -> Result<secp256k1::SecretKey> {
        let request = wire_bill::RequestToPayBitcreditBillPayload {
            bill_id,
            deadline,
            currency: CoreClient::currency_unit().to_string(),
            payment_address: payment_address.into_unchecked(),
        };
        let resp: wire_bill::RequestToPayBitcreditBillResponse =
            self.ebill.request_to_pay_bill(&request).await?;
        Ok(resp.bill_private_key)
    }

    async fn is_bill_paid(&self, bill_id: core::BillId) -> Result<bool> {
        let status = self.ebill.get_payment_status(bill_id).await?;
        Ok(status.payment_status.paid)
    }
}
