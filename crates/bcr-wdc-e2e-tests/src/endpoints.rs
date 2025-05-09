// ----- standard library imports
use std::marker::PhantomData;
// ----- extra library imports
use reqwest::Url;
// ----- local modules
// ----- end imports

pub struct Endpoints<T> {
    base_url: String,
    _marker: PhantomData<T>,
}

impl<T> Endpoints<T> {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            _marker: PhantomData,
        }
    }
    fn url(&self, path: &str) -> Url {
        Url::parse(&format!("{}/{}", self.base_url, path)).unwrap()
    }
}

pub struct UserService {}
pub struct AdminService {}

impl Endpoints<UserService> {
    pub fn mint_credit_quote_url(&self) -> Url {
        self.url("v1/mint/credit/quote")
    }
    pub fn lookup_credit_quote(&self, quote_id: &str) -> Url {
        self.url(&format!("v1/mint/credit/quote/{quote_id}"))
    }
    pub fn list_keysets(&self) -> Url {
        self.url("v1/keysets")
    }
    pub fn mint_ebill(&self) -> Url {
        self.url("v1/mint/ebill")
    }
}

impl Endpoints<AdminService> {
    pub fn keys_activate(&self) -> Url {
        self.url("v1/admin/keys/activate")
    }
    pub fn admin_credit_quote(&self, quote_id: &str) -> Url {
        self.url(&format!("v1/admin/credit/quote/{quote_id}"))
    }
}
