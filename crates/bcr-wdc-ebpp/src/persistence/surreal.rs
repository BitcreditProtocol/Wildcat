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
use cdk_common::payment::PaymentIdentifier;
use surrealdb::{engine::any::Any, Result as SurrealResult, Surreal};
// ----- local imports
use crate::{
    error::{Error, Result},
    onchain::{PrivateKeysRepository, SingleSecretKeyDescriptor},
    payment::{ForeignPayment, IncomingRequest, OutgoingRequest, PaymentType},
    service::PaymentRepository,
};

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
    reqid: PaymentIdentifier,
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
    reqid: PaymentIdentifier,
    recipient: btc::Address<btc::address::NetworkUnchecked>,
    amount: btc::Amount,
    reserved_fees: btc::Amount,
    status: cdk_common::MeltQuoteState,
    proof: Option<btc::Txid>,
    total_spent: Option<btc::Amount>,
}
impl std::convert::From<OutgoingRequest> for OutgoingPaymentDBEntry {
    fn from(req: OutgoingRequest) -> Self {
        Self {
            reqid: req.reqid,
            amount: req.amount,
            reserved_fees: req.reserved_fees,
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
        reserved_fees: dbreq.reserved_fees,
        recipient,
        status: dbreq.status,
        proof: dbreq.proof,
        total_spent: dbreq.total_spent,
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ForeignPaymentDBEntry {
    reqid: PaymentIdentifier,
    nonce: String,
    amount: btc::Amount,
}
impl From<ForeignPayment> for ForeignPaymentDBEntry {
    fn from(fp: ForeignPayment) -> Self {
        Self {
            reqid: fp.reqid,
            nonce: fp.nonce,
            amount: fp.amount,
        }
    }
}
impl From<ForeignPaymentDBEntry> for ForeignPayment {
    fn from(dbfp: ForeignPaymentDBEntry) -> Self {
        Self {
            reqid: dbfp.reqid,
            nonce: dbfp.nonce,
            amount: dbfp.amount,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct PaymentConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub incoming_payments_table: String,
    pub outgoing_payments_table: String,
    pub foreign_payments_table: String,
}

#[derive(Debug, Clone)]
pub struct DBPayments {
    db: Surreal<surrealdb::engine::any::Any>,
    incoming_table: String,
    outgoing_table: String,
    foreign: String,
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
            foreign: cfg.foreign_payments_table,
            network,
        })
    }

    async fn load_incoming(
        &self,
        reqid: PaymentIdentifier,
    ) -> SurrealResult<Option<IncomingPaymentDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(&self.incoming_table, reqid.to_string());
        self.db.select(rid).await
    }

    async fn store_incoming(
        &self,
        request: IncomingPaymentDBEntry,
    ) -> SurrealResult<Option<IncomingPaymentDBEntry>> {
        let rid =
            surrealdb::RecordId::from_table_key(&self.incoming_table, request.reqid.to_string());
        self.db.insert(rid).content(request).await
    }

    async fn update_incoming(
        &self,
        request: IncomingPaymentDBEntry,
    ) -> SurrealResult<Option<IncomingPaymentDBEntry>> {
        let rid =
            surrealdb::RecordId::from_table_key(&self.incoming_table, request.reqid.to_string());
        self.db.update(rid).content(request).await
    }

    async fn load_outgoing(
        &self,
        reqid: PaymentIdentifier,
    ) -> SurrealResult<Option<OutgoingPaymentDBEntry>> {
        let rid = surrealdb::RecordId::from_table_key(&self.outgoing_table, reqid.to_string());
        self.db.select(rid).await
    }

    async fn store_outgoing(
        &self,
        request: OutgoingPaymentDBEntry,
    ) -> SurrealResult<Option<OutgoingPaymentDBEntry>> {
        let rid =
            surrealdb::RecordId::from_table_key(&self.outgoing_table, request.reqid.to_string());
        self.db.insert(rid).content(request).await
    }

    async fn update_outgoing(
        &self,
        request: OutgoingPaymentDBEntry,
    ) -> SurrealResult<Option<OutgoingPaymentDBEntry>> {
        let rid =
            surrealdb::RecordId::from_table_key(&self.outgoing_table, request.reqid.to_string());
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
    async fn load_incoming(&self, reqid: &PaymentIdentifier) -> Result<IncomingRequest> {
        let dbreq = self
            .load_incoming(reqid.clone())
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let dbreq = dbreq.ok_or(Error::PaymentRequestNotFound(reqid.clone()))?;
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
        let reqid = req.reqid.clone();
        let res: Option<IncomingPaymentDBEntry> = self
            .update_incoming(req.clone().into())
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        if res.is_none() {
            return Err(Error::PaymentRequestNotFound(reqid));
        }
        Ok(())
    }

    async fn load_outgoing(&self, reqid: &PaymentIdentifier) -> Result<OutgoingRequest> {
        let dbreq = self
            .load_outgoing(reqid.clone())
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        let dbreq = dbreq.ok_or(Error::PaymentRequestNotFound(reqid.clone()))?;
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
        let reqid = req.reqid.clone();
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

    async fn store_foreign(&self, foreign: ForeignPayment) -> Result<()> {
        let rid = surrealdb::RecordId::from_table_key(&self.foreign, foreign.reqid.to_string());
        let entry = ForeignPaymentDBEntry {
            reqid: foreign.reqid,
            nonce: foreign.nonce,
            amount: foreign.amount,
        };
        let _: Option<ForeignPaymentDBEntry> = self
            .db
            .insert(rid)
            .content(entry)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(())
    }
    async fn check_foreign_nonce(&self, nonce: &str) -> Result<Option<ForeignPayment>> {
        let entry: Option<ForeignPaymentDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE nonce == $nonce")
            .bind(("table", self.foreign.clone()))
            .bind(("nonce", nonce.to_string()))
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(entry.map(From::from))
    }
    async fn check_foreign_reqid(
        &self,
        reqid: &PaymentIdentifier,
    ) -> Result<Option<ForeignPayment>> {
        let rid = surrealdb::RecordId::from_table_key(&self.foreign, reqid.to_string());
        let entry: Option<ForeignPaymentDBEntry> = self
            .db
            .select(rid)
            .await
            .map_err(|e| Error::DB(anyhow!(e)))?;
        Ok(entry.map(From::from))
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
            foreign: String::from("foreign"),
        }
    }

    #[tokio::test]
    async fn list_unpaid() {
        let db = init_mem_db().await;

        let mut unpaids: Vec<PaymentIdentifier> = Default::default();
        let address = btc::Address::from_str("1thMirt546nngXqyPEz532S8fLwbozud8").unwrap();
        let unpaid1 = IncomingPaymentDBEntry {
            reqid: PaymentIdentifier::PaymentId(rand::random()),
            amount: btc::Amount::ZERO,
            expiration: None,
            payment_type: PaymentTypeDBEntry::OnChain(address.clone()),
            status: MintQuoteState::Unpaid,
        };
        unpaids.push(unpaid1.reqid.clone());
        let rid = RecordId::from_table_key(&db.incoming_table, unpaid1.reqid.to_string());
        let _: Option<IncomingPaymentDBEntry> = db.db.insert(rid).content(unpaid1).await.unwrap();

        let paid1 = IncomingPaymentDBEntry {
            reqid: PaymentIdentifier::PaymentId(rand::random()),
            amount: btc::Amount::ZERO,
            expiration: None,
            payment_type: PaymentTypeDBEntry::OnChain(address.clone()),
            status: MintQuoteState::Paid,
        };
        let rid = RecordId::from_table_key(&db.incoming_table, paid1.reqid.to_string());
        let _: Option<IncomingPaymentDBEntry> = db.db.insert(rid).content(paid1).await.unwrap();

        let unpaid2 = IncomingPaymentDBEntry {
            reqid: PaymentIdentifier::PaymentId(rand::random()),
            amount: btc::Amount::ZERO,
            expiration: None,
            payment_type: PaymentTypeDBEntry::OnChain(address.clone()),
            status: MintQuoteState::Unpaid,
        };
        unpaids.push(unpaid2.reqid.clone());
        let rid = RecordId::from_table_key(&db.incoming_table, unpaid2.reqid.to_string());
        let _: Option<IncomingPaymentDBEntry> = db.db.insert(rid).content(unpaid2).await.unwrap();

        let list = db.list_unpaid().await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(unpaids.contains(&list[0].reqid));
        assert!(unpaids.contains(&list[1].reqid));
    }

    #[tokio::test]
    async fn store_check_foreign() {
        let db = init_mem_db().await;
        let foreign = ForeignPayment {
            reqid: PaymentIdentifier::PaymentId(rand::random()),
            nonce: String::from("nonce"),
            amount: btc::Amount::from_sat(1000),
        };

        db.store_foreign(foreign.clone()).await.unwrap();
        let exists = db.check_foreign_nonce("nonce").await.unwrap();
        assert!(exists.is_some());
        let not_exists = db.check_foreign_nonce("other").await.unwrap();
        assert!(not_exists.is_none());

        let exists_reqid = db.check_foreign_reqid(&foreign.reqid).await.unwrap();
        assert!(exists_reqid.is_some());
        let not_exists_reqid = db
            .check_foreign_reqid(&PaymentIdentifier::PaymentId(rand::random()))
            .await
            .unwrap();
        assert!(not_exists_reqid.is_none());
    }
}
