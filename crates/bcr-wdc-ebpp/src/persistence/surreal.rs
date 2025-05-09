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
use cashu::MintQuoteState;
use surrealdb::{engine::any::Any, Result as SurrealResult, Surreal};
use uuid::Uuid;
// ----- local imports
use crate::error::{Error, Result};
use crate::onchain::{PrivateKeysRepository, SingleSecretKeyDescriptor};
use crate::payment::{IncomingRequest, OutgoingRequest, PaymentType};
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
struct IncomingPaymentDBEntry {
    reqid: Uuid,
    amount: btc::Amount,
    payment_type: PaymentTypeDBEntry,
    status: cdk_common::MintQuoteState,
    expiration: Option<chrono::DateTime<chrono::Utc>>,
}
impl std::convert::From<IncomingRequest> for IncomingPaymentDBEntry {
    fn from(req: IncomingRequest) -> Self {
        Self {
            reqid: req.reqid,
            amount: req.amount,
            payment_type: req.payment_type.into(),
            status: req.status,
            expiration: req.expiration,
        }
    }
}
fn into_incoming_request(
    dbreq: IncomingPaymentDBEntry,
    network: btc::Network,
) -> Result<IncomingRequest> {
    let ttype = into_payment_type(dbreq.payment_type, network)?;
    Ok(IncomingRequest {
        reqid: dbreq.reqid,
        amount: dbreq.amount,
        payment_type: ttype,
        status: dbreq.status,
        expiration: dbreq.expiration,
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct OutgoingPaymentDBEntry {
    reqid: Uuid,
    recipient: btc::Address<btc::address::NetworkUnchecked>,
    amount: btc::Amount,
    status: cdk_common::MeltQuoteState,
    proof: Option<btc::Txid>,
    total_spent: Option<btc::Amount>,
}
impl std::convert::From<OutgoingRequest> for OutgoingPaymentDBEntry {
    fn from(req: OutgoingRequest) -> Self {
        Self {
            reqid: req.reqid,
            amount: req.amount,
            status: req.status,
            recipient: req.recipient.into_unchecked(),
            proof: req.proof,
            total_spent: req.total_spent,
        }
    }
}
fn into_outgoing_request(
    dbreq: OutgoingPaymentDBEntry,
    network: btc::Network,
) -> Result<OutgoingRequest> {
    let recipient = dbreq.recipient.require_network(network)?;
    Ok(OutgoingRequest {
        reqid: dbreq.reqid,
        amount: dbreq.amount,
        recipient,
        status: dbreq.status,
        proof: dbreq.proof,
        total_spent: dbreq.total_spent,
    })
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct PaymentConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub incoming_payments_table: String,
    pub outgoing_payments_table: String,
}

#[derive(Debug, Clone)]
pub struct DBPayments {
    db: Surreal<surrealdb::engine::any::Any>,
    incoming_table: String,
    outgoing_table: String,
    network: btc::Network,
}

impl DBPayments {
    pub async fn new(cfg: PaymentConnectionConfig, network: btc::Network) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self {
            db: db_connection,
            incoming_table: cfg.incoming_payments_table,
            outgoing_table: cfg.outgoing_payments_table,
            network,
        })
    }

    async fn load_incoming(&self, reqid: Uuid) -> SurrealResult<Option<IncomingPaymentDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(&self.incoming_table, reqid);
        self.db.select(rid).await
    }

    async fn store_incoming(
        &self,
        request: IncomingPaymentDBEntry,
    ) -> SurrealResult<Option<IncomingPaymentDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(&self.incoming_table, request.reqid);
        self.db.insert(rid).content(request).await
    }

    async fn update_incoming(
        &self,
        request: IncomingPaymentDBEntry,
    ) -> SurrealResult<Option<IncomingPaymentDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(&self.incoming_table, request.reqid);
        self.db.update(rid).content(request).await
    }

    async fn load_outgoing(&self, reqid: Uuid) -> SurrealResult<Option<OutgoingPaymentDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(&self.outgoing_table, reqid);
        self.db.select(rid).await
    }

    async fn store_outgoing(
        &self,
        request: OutgoingPaymentDBEntry,
    ) -> SurrealResult<Option<OutgoingPaymentDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(&self.outgoing_table, request.reqid);
        self.db.insert(rid).content(request).await
    }

    async fn update_outgoing(
        &self,
        request: OutgoingPaymentDBEntry,
    ) -> SurrealResult<Option<OutgoingPaymentDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(&self.outgoing_table, request.reqid);
        self.db.update(rid).content(request).await
    }

    async fn list_unpaid(&self) -> SurrealResult<Vec<IncomingPaymentDBEntry>> {
        let statement = "SELECT * FROM type::table($table) WHERE status == $status";
        self.db
            .query(statement)
            .bind(("table", self.incoming_table.clone()))
            .bind(("status", MintQuoteState::Unpaid))
            .await?
            .take(0)
    }
}

#[async_trait]
impl PaymentRepository for DBPayments {
    async fn load_incoming(&self, reqid: Uuid) -> Result<IncomingRequest> {
        let dbreq = self
            .load_incoming(reqid)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let dbreq = dbreq.ok_or(Error::PaymentRequestNotFound(reqid))?;
        into_incoming_request(dbreq, self.network)
    }

    async fn store_incoming(&self, req: IncomingRequest) -> Result<()> {
        let dbreq = IncomingPaymentDBEntry::from(req);
        self.store_incoming(dbreq)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }

    async fn update_incoming(&self, req: IncomingRequest) -> Result<()> {
        let reqid = req.reqid;
        let res: Option<IncomingPaymentDBEntry> = self
            .update_incoming(req.into())
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        if res.is_none() {
            return Err(Error::PaymentRequestNotFound(reqid));
        }
        Ok(())
    }

    async fn load_outgoing(&self, reqid: Uuid) -> Result<OutgoingRequest> {
        let dbreq = self
            .load_outgoing(reqid)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let dbreq = dbreq.ok_or(Error::PaymentRequestNotFound(reqid))?;
        into_outgoing_request(dbreq, self.network)
    }

    async fn store_outgoing(&self, req: OutgoingRequest) -> Result<()> {
        let dbreq = OutgoingPaymentDBEntry::from(req);
        self.store_outgoing(dbreq)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }
    async fn update_outgoing(&self, req: OutgoingRequest) -> Result<()> {
        let reqid = req.reqid;
        let res: Option<OutgoingPaymentDBEntry> = self
            .update_outgoing(req.into())
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        if res.is_none() {
            return Err(Error::PaymentRequestNotFound(reqid));
        }
        Ok(())
    }

    async fn list_unpaid_incoming_requests(&self) -> Result<Vec<IncomingRequest>> {
        let result = self
            .list_unpaid()
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let requests = result
            .into_iter()
            .map(|dbentry| into_incoming_request(dbentry, self.network))
            .collect::<Result<_>>()?;
        Ok(requests)
    }
}

#[cfg(test)]
mod tests {
    use surrealdb::RecordId;

    use super::*;

    async fn init_mem_db() -> DBPayments {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBPayments {
            db: sdb,
            network: btc::Network::Bitcoin,
            incoming_table: String::from("incoming"),
            outgoing_table: String::from("outgoing"),
        }
    }

    #[tokio::test]
    async fn list_unpaid() {
        let db = init_mem_db().await;

        let mut unpaids: [uuid::Uuid; 2] = Default::default();
        let address = btc::Address::from_str("1thMirt546nngXqyPEz532S8fLwbozud8").unwrap();
        let unpaid1 = IncomingPaymentDBEntry {
            reqid: Uuid::new_v4(),
            amount: btc::Amount::ZERO,
            expiration: None,
            payment_type: PaymentTypeDBEntry::OnChain(address.clone()),
            status: MintQuoteState::Unpaid,
        };
        unpaids[0] = unpaid1.reqid;
        let rid = RecordId::from_table_key(&db.incoming_table, unpaid1.reqid);
        let _: Option<IncomingPaymentDBEntry> = db.db.insert(rid).content(unpaid1).await.unwrap();

        let paid1 = IncomingPaymentDBEntry {
            reqid: Uuid::new_v4(),
            amount: btc::Amount::ZERO,
            expiration: None,
            payment_type: PaymentTypeDBEntry::OnChain(address.clone()),
            status: MintQuoteState::Paid,
        };
        let rid = RecordId::from_table_key(&db.incoming_table, paid1.reqid);
        let _: Option<IncomingPaymentDBEntry> = db.db.insert(rid).content(paid1).await.unwrap();

        let unpaid2 = IncomingPaymentDBEntry {
            reqid: Uuid::new_v4(),
            amount: btc::Amount::ZERO,
            expiration: None,
            payment_type: PaymentTypeDBEntry::OnChain(address.clone()),
            status: MintQuoteState::Unpaid,
        };
        unpaids[1] = unpaid2.reqid;
        let rid = RecordId::from_table_key(&db.incoming_table, unpaid2.reqid);
        let _: Option<IncomingPaymentDBEntry> = db.db.insert(rid).content(unpaid2).await.unwrap();

        let mut list = db.list_unpaid().await.unwrap();
        assert_eq!(list.len(), 2);
        list.sort_by(|a, b| a.reqid.cmp(&b.reqid));
        unpaids.sort();
        assert_eq!(list[0].reqid, unpaids[0]);
        assert_eq!(list[1].reqid, unpaids[1]);
    }
}
