// ----- standard library imports
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
// ----- extra library imports
use uuid::Uuid;
// ----- local modules
use super::mint::{Quote, QuoteRepository};

#[derive(Default, Clone)]
pub struct InMemoryQuoteRepository {
    quotes: Arc<RwLock<HashMap<Uuid, Quote>>>,
}
impl QuoteRepository for InMemoryQuoteRepository {
    fn load(&self, id: Uuid) -> Option<Quote> {
        self.quotes.read().unwrap().get(&id).cloned()
    }

    fn list(&self) -> Vec<Uuid> {
        self.quotes.read().unwrap().keys().cloned().collect()
    }

    fn store(&self, quote: Quote) {
        let id = match &quote {
            Quote::Pending(request) => request.id,
            Quote::Declined(request) => request.id,
            Quote::Accepted(request, ..) => request.id,
        };
        self.quotes.write().unwrap().insert(id, quote);
    }

    fn remove(&self, id: Uuid) -> Option<Quote> {
        self.quotes.write().unwrap().remove(&id)
    }
}
