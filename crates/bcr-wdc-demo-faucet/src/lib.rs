use std::collections::HashMap;

use bcr_wdc_quote_client::bcr_wdc_webapi;
// ----- standard library imports
// ----- extra library imports
use bcr_wdc_webapi::quotes as web_quotes;
// ----- local modules

// ----- end imports

type TStamp = chrono::DateTime<chrono::Utc>;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    quote_url: bcr_wdc_quote_client::Url,
    key_url: bcr_wdc_key_client::Url,
    ebill_url: bcr_wdc_ebill_client::Url,
    authorized_drawees: Vec<bcr_ebill_core::NodeId>,
    sleep_secs: u64,
    retention_period_min: i64,
    discount: u64,
}

pub async fn main_loop(
    cfg: AppConfig,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<(), Box<dyn std::error::Error>> {
    let AppConfig {
        quote_url,
        ebill_url,
        key_url,
        authorized_drawees,
        sleep_secs,
        retention_period_min,
        discount,
    } = cfg;
    let sleep = tokio::time::Duration::from_secs(sleep_secs);
    let retention_period = chrono::Duration::minutes(retention_period_min);
    let quote_client = bcr_wdc_quote_client::QuoteClient::new(quote_url);
    let key_client = bcr_wdc_key_client::KeyClient::new(key_url);
    let ebill_client = bcr_wdc_ebill_client::EbillClient::new(ebill_url);

    let mut activity_log: HashMap<bcr_ebill_core::NodeId, TStamp> = HashMap::new();
    let myself = ebill_client.get_identity().await?;

    loop {
        // check for pending quotes
        let params = web_quotes::ListParam {
            status: Some(web_quotes::StatusReplyDiscriminants::Pending),
            ..Default::default()
        };
        let light_quotes = quote_client.list(params).await?;
        for light in light_quotes.quotes {
            let quote = quote_client.info(light.id).await?;
            let web_quotes::InfoReply::Pending {
                bill, submitted, ..
            } = quote
            else {
                tracing::warn!("Unexpected quote status: {:?}", quote);
                continue;
            };
            // validate the original drawee
            let drawee_id = bill.drawee.node_id;
            if !authorized_drawees.contains(&drawee_id) {
                quote_client
                    .update(light.id, web_quotes::UpdateQuoteRequest::Deny)
                    .await?;
                continue;
            }
            // validate retention period of the current holder
            let holder_id = bill.endorsees.last().unwrap_or(&bill.payee).node_id();
            let last_activity = activity_log
                .get(&holder_id)
                .cloned()
                .unwrap_or(TStamp::MIN_UTC);
            if submitted - last_activity < retention_period {
                quote_client
                    .update(light.id, web_quotes::UpdateQuoteRequest::Deny)
                    .await?;
                continue;
            }

            activity_log.insert(holder_id, submitted);

            let discounted = bitcoin::Amount::from_sat(bill.sum - discount);
            let offer = web_quotes::UpdateQuoteRequest::Offer {
                discounted,
                ttl: None,
            };
            quote_client.update(light.id, offer).await?;
        }

        // check for accepted quotes
        let params = web_quotes::ListParam {
            status: Some(web_quotes::StatusReplyDiscriminants::Accepted),
            ..Default::default()
        };
        let light_quotes = quote_client.list(params).await?;
        for light in light_quotes.quotes {
            let quote = quote_client.info(light.id).await?;
            let web_quotes::InfoReply::Accepted { id, bill, .. } = quote else {
                tracing::warn!("Unexpected quote status: {:?}", quote);
                continue;
            };

            let ebill = ebill_client.get_bill(&bill.id).await?;
            let holder_id = ebill
                .participants
                .endorsee
                .unwrap_or(ebill.participants.payee)
                .node_id();
            if myself.node_id != holder_id {
                tracing::warn!("Skipping quote {} for bill {}: not the holder", id, bill.id);
                continue;
            }

            key_client.enable_keyset(light.id).await?;
        }

        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("Cancellation requested, exiting main loop.");
                break;
            },
            _ = tokio::time::sleep(sleep) => {
                tracing::info!("Waiting for the next iteration...");
            }
        }
    }

    Ok(())
}
