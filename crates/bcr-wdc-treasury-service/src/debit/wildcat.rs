// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    client::{
        keys::{Client as KeysClient, Error as KeysError},
        quote::Client as QuoteClient,
        swap::Client as SwapClient,
    },
    core::BillId,
    wire::quotes as wire_quotes,
};
use cashu::{nut00 as cdk00, nut02 as cdk02};
// ----- local imports
use crate::{
    debit::service::WildcatService,
    error::{Error, Result},
};

// ----- end imports

#[derive(Clone, Debug, serde::Deserialize)]
pub struct WildcatClientConfig {
    pub swap_service_url: reqwest::Url,
    pub quote_service_url: reqwest::Url,
    pub key_service_url: reqwest::Url,
}

#[derive(Clone, Debug)]
pub struct WildcatCl {
    swap_cl: SwapClient,
    quote_cl: QuoteClient,
    key_cl: KeysClient,
}

impl WildcatCl {
    pub fn new(cfg: WildcatClientConfig) -> Self {
        let swap_cl = SwapClient::new(cfg.swap_service_url);
        let quote_cl = QuoteClient::new(cfg.quote_service_url);
        let key_cl = KeysClient::new(cfg.key_service_url);
        Self {
            swap_cl,
            quote_cl,
            key_cl,
        }
    }
}

#[async_trait]
impl WildcatService for WildcatCl {
    async fn burn(&self, inputs: &[cdk00::Proof]) -> Result<()> {
        self.swap_cl.burn(inputs.to_vec()).await?;
        Ok(())
    }
    async fn deactivate_keyset_for_ebill(&self, ebill_id: &BillId) -> Result<cdk02::Id> {
        // find all quotes for the ebill
        let params = wire_quotes::ListParam {
            bill_id: Some(ebill_id.clone()),
            ..Default::default()
        };
        let list = self.quote_cl.list(params).await?;
        // filter by status that contains keyset_id, from the most to the least likely
        for status in [
            wire_quotes::StatusReplyDiscriminants::Accepted,
            wire_quotes::StatusReplyDiscriminants::Offered,
            wire_quotes::StatusReplyDiscriminants::OfferExpired,
        ] {
            let candidates: Vec<_> = list
                .quotes
                .iter()
                .filter(|light| light.status == status)
                .collect();
            if candidates.len() > 1 {
                tracing::warn!(
                    "Multiple quotes with status {status} found for ebill {ebill_id}: {}",
                    candidates.len()
                );
            }
            for candidate in candidates {
                let quote = self.quote_cl.lookup(candidate.id).await?;
                let Some(kid) = extract_keyset_id(quote) else {
                    continue;
                };
                let resp = self.key_cl.deactivate_keyset(kid).await;
                match resp {
                    Ok(_) => return Ok(kid),
                    Err(KeysError::KeysetIdNotFound(_)) => {
                        continue;
                    }
                    Err(e) => return Err(Error::KeyClient(e)),
                }
            }
        }
        Err(Error::EBillNotFound(ebill_id.to_string()))
    }

    async fn keyset_info(&self, kid: cdk02::Id) -> Result<cdk02::KeySetInfo> {
        let info = self.key_cl.keyset_info(kid).await?;
        Ok(info)
    }
}

fn extract_keyset_id(quote: wire_quotes::StatusReply) -> Option<cdk02::Id> {
    match quote {
        wire_quotes::StatusReply::Accepted { keyset_id, .. } => Some(keyset_id),
        wire_quotes::StatusReply::Offered { keyset_id, .. } => Some(keyset_id),
        _ => None,
    }
}
