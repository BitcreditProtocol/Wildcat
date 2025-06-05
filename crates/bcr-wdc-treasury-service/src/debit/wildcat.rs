// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_key_client as key;
use bcr_wdc_quote_client as quote;
use bcr_wdc_swap_client as swap;
use bcr_wdc_webapi::quotes as web_quotes;
use cashu::{nut00 as cdk00, nut02 as cdk02};
// ----- local imports
use crate::{
    debit::service::WildcatService,
    error::{Error, Result},
};

// ----- end imports

#[derive(Clone, Debug, serde::Deserialize)]
pub struct WildcatClientConfig {
    pub swap_service_url: swap::Url,
    pub quote_service_url: quote::Url,
    pub key_service_url: key::Url,
}

#[derive(Clone, Debug)]
pub struct WildcatCl {
    swap_cl: swap::SwapClient,
    quote_cl: quote::QuoteClient,
    key_cl: key::KeyClient,
}

impl WildcatCl {
    pub fn new(cfg: WildcatClientConfig) -> Self {
        let swap_cl = swap::SwapClient::new(cfg.swap_service_url);
        let quote_cl = quote::QuoteClient::new(cfg.quote_service_url);
        let key_cl = key::KeyClient::new(cfg.key_service_url);
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
    async fn deactivate_keyset_for_ebill(&self, ebill_id: &str) -> Result<cdk02::Id> {
        // find all quotes for the ebill
        let params = web_quotes::ListParam {
            bill_id: Some(ebill_id.to_string()),
            ..Default::default()
        };
        let list = self.quote_cl.list(params).await?;
        // filter by status that contains keyset_id, from the most to the least likely
        for status in [
            web_quotes::StatusReplyDiscriminants::Accepted,
            web_quotes::StatusReplyDiscriminants::Offered,
            web_quotes::StatusReplyDiscriminants::OfferExpired,
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
                    Err(key::Error::ResourceNotFound(_)) => {
                        continue;
                    }
                    Err(e) => return Err(Error::KeyClient(e)),
                }
            }
        }
        Err(Error::EBillNotFound(ebill_id.to_string()))
    }
}

fn extract_keyset_id(quote: web_quotes::StatusReply) -> Option<cdk02::Id> {
    match quote {
        web_quotes::StatusReply::Accepted { keyset_id, .. } => Some(keyset_id),
        web_quotes::StatusReply::Offered { keyset_id, .. } => Some(keyset_id),
        _ => None,
    }
}
