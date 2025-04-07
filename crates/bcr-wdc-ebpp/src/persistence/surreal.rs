// ----- standard library imports
use std::collections::HashMap;
use std::str::FromStr;
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bdk_wallet::miniscript::{
    bitcoin::{
        self as btc,
        hashes::{sha256, Hash},
    },
    descriptor::{DescriptorSecretKey, KeyMap},
    Descriptor, DescriptorPublicKey,
};
use surrealdb::{engine::any::Any, Result as SurrealResult, Surreal};
use uuid::Uuid;
// ----- local imports
use crate::error::{Error, Result};
use crate::onchain::{PrivateKeysRepository, SingleSecretKeyDescriptor};
use crate::payment::{PaymentType, Request};
use crate::service::PaymentRepository;

// ----- end imports

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KeysDBEntry {
    desc: Descriptor<DescriptorPublicKey>,
    kmap: HashMap<DescriptorPublicKey, String>, // DescriptorSecretKey
}

impl From<(Descriptor<DescriptorPublicKey>, KeyMap)> for KeysDBEntry {
    fn from(ke: SingleSecretKeyDescriptor) -> Self {
        let (desc, kmap) = ke;
        let mut serialized_keys: HashMap<_, _> = Default::default();
        for (k, v) in kmap {
            serialized_keys.insert(k, v.to_string());
        }
        Self {
            desc,
            kmap: serialized_keys,
        }
    }
}

impl From<KeysDBEntry> for SingleSecretKeyDescriptor {
    fn from(dbk: KeysDBEntry) -> Self {
        let KeysDBEntry { desc, kmap } = dbk;
        let mut keysmap: KeyMap = Default::default();
        for (k, v) in kmap {
            keysmap.insert(k, DescriptorSecretKey::from_str(&v).unwrap());
        }
        (desc, keysmap)
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub table: String,
}

#[derive(Debug, Clone)]
pub struct DBPrivateKeys {
    db: Surreal<surrealdb::engine::any::Any>,
    table: String,
}

impl DBPrivateKeys {
    pub async fn new(cfg: ConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self {
            db: db_connection,
            table: cfg.table,
        })
    }

    async fn list_keys(&self) -> SurrealResult<Vec<KeysDBEntry>> {
        self.db
            .query("SELECT * FROM type::table($table)")
            .bind(("table", self.table.clone()))
            .await?
            .take(0)
    }
}

#[async_trait]
impl PrivateKeysRepository for DBPrivateKeys {
    async fn get_private_keys(&self) -> Result<Vec<SingleSecretKeyDescriptor>> {
        let dbkeys = self.list_keys().await.map_err(|e| Error::DB(anyhow!(e)))?;
        let keys = dbkeys.into_iter().map(From::from).collect();
        Ok(keys)
    }

    async fn add_key(&self, key: SingleSecretKeyDescriptor) -> Result<()> {
        let rkey = sha256::Hash::hash(key.0.to_string().as_bytes()).to_string();
        let rid = surrealdb::RecordId::from_table_key(&self.table, rkey);
        let dbkey = KeysDBEntry::from(key);
        let _resp: Option<KeysDBEntry> = self
            .db
            .insert(rid)
            .content(dbkey)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum PaymentTypeDBEntry {
    OnChain(btc::Address<btc::address::NetworkUnchecked>),
    EBill(btc::Address<btc::address::NetworkUnchecked>),
}
impl std::convert::From<PaymentType> for PaymentTypeDBEntry {
    fn from(pt: PaymentType) -> Self {
        match pt {
            PaymentType::OnChain(addr) => PaymentTypeDBEntry::OnChain(addr.into_unchecked()),
            PaymentType::EBill(addr) => PaymentTypeDBEntry::EBill(addr.into_unchecked()),
        }
    }
}
fn into_payment_type(pt: PaymentTypeDBEntry, network: btc::Network) -> Result<PaymentType> {
    match pt {
        PaymentTypeDBEntry::OnChain(addr) => {
            let addr = addr
                .require_network(network)
                .map_err(Error::BTCAddressParse)?;
            Ok(PaymentType::OnChain(addr))
        }
        PaymentTypeDBEntry::EBill(addr) => {
            let addr = addr
                .require_network(network)
                .map_err(Error::BTCAddressParse)?;
            Ok(PaymentType::EBill(addr))
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PaymentRequestDBEntry {
    reqid: Uuid,
    amount: cashu::Amount,
    currency: cashu::CurrencyUnit,
    payment_type: PaymentTypeDBEntry,
    status: cdk_common::MintQuoteState,
}
impl std::convert::From<Request> for PaymentRequestDBEntry {
    fn from(req: Request) -> Self {
        Self {
            reqid: req.reqid,
            amount: req.amount,
            currency: req.currency,
            payment_type: req.payment_type.into(),
            status: req.status,
        }
    }
}
fn into_request(dbreq: PaymentRequestDBEntry, network: btc::Network) -> Result<Request> {
    let ttype = into_payment_type(dbreq.payment_type, network)?;
    Ok(Request {
        reqid: dbreq.reqid,
        amount: dbreq.amount,
        currency: dbreq.currency,
        payment_type: ttype,
        status: dbreq.status,
    })
}

#[derive(Debug, Clone)]
pub struct DBPayments {
    db: Surreal<surrealdb::engine::any::Any>,
    table: String,
    network: btc::Network,
}

impl DBPayments {
    pub async fn new(cfg: ConnectionConfig, network: btc::Network) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self {
            db: db_connection,
            table: cfg.table,
            network,
        })
    }

    async fn load(&self, reqid: Uuid) -> SurrealResult<Option<PaymentRequestDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(&self.table, reqid);
        self.db.select(rid).await
    }

    async fn store(
        &self,
        request: PaymentRequestDBEntry,
    ) -> SurrealResult<Option<PaymentRequestDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(&self.table, request.reqid);
        self.db.insert(rid).content(request).await
    }

    async fn update(
        &self,
        request: PaymentRequestDBEntry,
    ) -> SurrealResult<Option<PaymentRequestDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(&self.table, request.reqid);
        self.db.update(rid).content(request).await
    }
}

#[async_trait]
impl PaymentRepository for DBPayments {
    async fn load_request(&self, reqid: Uuid) -> Result<Request> {
        let dbreq = self.load(reqid).await.map_err(|e| Error::DB(anyhow!(e)))?;
        let dbreq = dbreq.ok_or(Error::PaymentRequestNotFound(reqid))?;
        into_request(dbreq, self.network)
    }

    async fn store_request(&self, req: Request) -> Result<()> {
        let dbreq = PaymentRequestDBEntry::from(req);
        self.store(dbreq).await.map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn update_request(&self, req: Request) -> Result<()> {
        let reqid = req.reqid;
        let res: Option<PaymentRequestDBEntry> = self
            .update(req.into())
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        if res.is_none() {
            return Err(Error::PaymentRequestNotFound(reqid));
        }
        Ok(())
    }
}
