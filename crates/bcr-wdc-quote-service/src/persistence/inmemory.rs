// ----- standard library imports
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
// ----- extra library imports
use anyhow::Result as AnyResult;
use async_trait::async_trait;
use strum::IntoDiscriminant;
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::quotes;
use crate::service::{ListFilters, Repository, SortOrder};
use crate::TStamp;

#[derive(Default, Clone)]
pub struct QuotesIDMap {
    quotes: Arc<RwLock<HashMap<Uuid, quotes::Quote>>>,
}
#[async_trait]
impl Repository for QuotesIDMap {
    async fn search_by_bill(&self, bill: &str, endorser: &str) -> AnyResult<Vec<quotes::Quote>> {
        Ok(self
            .quotes
            .read()
            .unwrap()
            .iter()
            .filter(|quote| quote.1.bill.id == bill && quote.1.bill.holder.node_id == endorser)
            .map(|x| x.1.clone())
            .collect())
    }

    async fn store(&self, quote: quotes::Quote) -> AnyResult<()> {
        self.quotes.write().unwrap().insert(quote.id, quote);
        Ok(())
    }
    async fn load(&self, id: uuid::Uuid) -> AnyResult<Option<quotes::Quote>> {
        Ok(self.quotes.read().unwrap().get(&id).cloned())
    }

    async fn update_if_pending(&self, new: quotes::Quote) -> AnyResult<()> {
        let id = new.id;
        let mut m = self.quotes.write().unwrap();
        let result = m.remove(&id);
        if let Some(old) = result {
            if matches!(old.status, quotes::QuoteStatus::Pending { .. }) {
                m.insert(id, new);
            } else {
                m.insert(id, old);
            }
        }
        Ok(())
    }

    async fn update_if_offered(&self, new: quotes::Quote) -> AnyResult<()> {
        let id = new.id;
        let mut m = self.quotes.write().unwrap();
        let result = m.remove(&id);
        if let Some(old) = result {
            if matches!(old.status, quotes::QuoteStatus::Offered { .. }) {
                m.insert(id, new);
            } else {
                m.insert(id, old);
            }
        }
        Ok(())
    }

    async fn list_pendings(&self, since: Option<TStamp>) -> AnyResult<Vec<Uuid>> {
        let a = self
            .quotes
            .read()
            .unwrap()
            .iter()
            .filter(|(_, q)| matches!(q.status, quotes::QuoteStatus::Pending { .. }))
            .filter(|(_, q)| q.submitted >= since.unwrap_or_default())
            .map(|(id, _)| *id)
            .collect();
        Ok(a)
    }
    async fn list_light(
        &self,
        filters: ListFilters,
        sort: Option<SortOrder>,
    ) -> AnyResult<Vec<quotes::LightQuote>> {
        let mut a: Vec<quotes::Quote> = self
            .quotes
            .read()
            .unwrap()
            .iter()
            .filter(|quote| {
                let ListFilters {
                    bill_maturity_date_from,
                    bill_maturity_date_to,
                    status,
                    bill_drawee_id,
                    bill_drawer_id,
                    bill_payer_id,
                    bill_holder_id,
                } = &filters;
                if let Some(bill_maturity_date_from) = bill_maturity_date_from {
                    if quote.1.bill.maturity_date.date_naive() < *bill_maturity_date_from {
                        return false;
                    }
                }
                if let Some(bill_maturity_date_to) = bill_maturity_date_to {
                    if quote.1.bill.maturity_date.date_naive() > *bill_maturity_date_to {
                        return false;
                    }
                }
                if let Some(status) = status {
                    if quote.1.status.discriminant() != *status {
                        return false;
                    }
                }
                if let Some(bill_drawee_id) = bill_drawee_id {
                    if quote.1.bill.drawee.node_id != *bill_drawee_id {
                        return false;
                    }
                }
                if let Some(bill_drawer_id) = bill_drawer_id {
                    if quote.1.bill.drawer.node_id != *bill_drawer_id {
                        return false;
                    }
                }
                if let Some(bill_payer_id) = bill_payer_id {
                    if quote.1.bill.payer.node_id != *bill_payer_id {
                        return false;
                    }
                }
                if let Some(bill_holder_id) = bill_holder_id {
                    if quote.1.bill.holder.node_id != *bill_holder_id {
                        return false;
                    }
                }
                return true;
            })
            .map(|(_, quote)| quote.clone())
            .collect();
        if let Some(sort) = sort {
            a.sort_by(|q1, q2| match sort {
                SortOrder::BillMaturityDateAsc => q1.bill.maturity_date.cmp(&q2.bill.maturity_date),
                SortOrder::BillMaturityDateDesc => {
                    q2.bill.maturity_date.cmp(&q1.bill.maturity_date)
                }
            });
        }
        let b = a
            .into_iter()
            .map(|quote| quotes::LightQuote {
                id: quote.id,
                status: quote.status.discriminant(),
            })
            .collect();
        Ok(b)
    }
}
