// ----- standard library imports
use std::collections::{BTreeMap, HashMap};
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_wdc_utils::keys::KeysetEntry;
use cashu::{nut00 as cdk00, nut01 as cdk01, nut02 as cdk02, nut12 as cdk12, Amount, PublicKey};
use cdk_common::mint::MintKeySetInfo;
use surrealdb::{
    engine::any::Any, error::Db as SurrealDBError, Error as SurrealError, RecordId,
    Result as SurrealResult, Surreal,
};
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    service::{KeysRepository, MintOperation, SignaturesRepository},
    TStamp,
};

// ----- end imports

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KeysDBEntry {
    id: RecordId,
    info: MintKeySetInfo,
    // unpacking MintKeySet because surrealdb doesn't support BTreeMap<K,V> where K is not a String
    unit: cdk00::CurrencyUnit,
    keys: HashMap<String, cdk01::MintKeyPair>,
    final_expiry: Option<u64>,
}

fn convert_to_keysdbentry(entry: KeysetEntry, table: &str) -> KeysDBEntry {
    let (info, keyset) = entry;
    let id = RecordId::from_table_key(table, info.id.to_string());
    let mut serialized_keys = HashMap::new();
    let cdk02::MintKeySet { unit, mut keys, .. } = keyset;
    while let Some((amount, keypair)) = keys.pop_last() {
        // surrealDB does not accept map with keys of type anything but Strings
        // so we need to serialize the keys to strings...
        serialized_keys.insert(amount.to_string(), keypair);
    }
    KeysDBEntry {
        id,
        info,
        unit,
        keys: serialized_keys,
        final_expiry: keyset.final_expiry,
    }
}
impl std::convert::From<KeysDBEntry> for KeysetEntry {
    fn from(dbk: KeysDBEntry) -> Self {
        let KeysDBEntry {
            info,
            unit,
            keys,
            final_expiry,
            ..
        } = dbk;
        let mut keysmap: BTreeMap<Amount, cdk01::MintKeyPair> = BTreeMap::default();
        for (val, keypair) in keys.into_iter() {
            // ... and parse them back to the original type
            let uval = val.parse::<u64>().expect("Failed to parse amount");
            keysmap.insert(Amount::from(uval), keypair);
        }
        let keyset = cdk02::MintKeySet {
            id: info.id,
            unit,
            keys: cdk01::MintKeys::new(keysmap),
            final_expiry,
        };
        (info, keyset)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MintOpDBEntry {
    id: RecordId,
    kid: cashu::Id,
    pub_key: cashu::PublicKey,
    target: cashu::Amount,
    minted: cashu::Amount,
}

fn convert_to_mintopdbentry(entry: MintOperation, table: &str) -> MintOpDBEntry {
    let MintOperation {
        uid,
        kid,
        pub_key,
        target,
        minted,
    } = entry;
    let id = RecordId::from_table_key(table, uid);
    MintOpDBEntry {
        id,
        kid,
        pub_key,
        target,
        minted,
    }
}
impl std::convert::From<MintOpDBEntry> for MintOperation {
    fn from(entry: MintOpDBEntry) -> Self {
        let key = entry.id.key();
        let uid = Uuid::try_from(key.clone()).expect("key is a uuid");
        Self {
            uid,
            kid: entry.kid,
            pub_key: entry.pub_key,
            target: entry.target,
            minted: entry.minted,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct DBKeysConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub keys_table: String,
    pub mints_table: String,
}

#[derive(Debug, Clone)]
pub struct DBKeys {
    db: Surreal<surrealdb::engine::any::Any>,
    keys_table: String,
    mints_table: String,
}

impl DBKeys {
    pub async fn new(cfg: DBKeysConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self {
            db: db_connection,
            keys_table: cfg.keys_table,
            mints_table: cfg.mints_table,
        })
    }
    async fn entry(&self, rid: RecordId) -> SurrealResult<Option<KeysetEntry>> {
        let response: Option<KeysDBEntry> = self.db.select(rid).await?;
        let Some(keysdbentry) = response else {
            return Ok(None);
        };
        let entry = KeysetEntry::from(keysdbentry);
        Ok(Some(entry))
    }
}

#[async_trait]
impl KeysRepository for DBKeys {
    async fn store(&self, entry: KeysetEntry) -> Result<()> {
        let rid = RecordId::from_table_key(self.keys_table.clone(), entry.0.id.to_string());
        let dbentry = convert_to_keysdbentry(entry, &self.keys_table);
        let _resp: Option<KeysDBEntry> = self
            .db
            .insert(rid)
            .content(dbentry)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        Ok(())
    }
    async fn info(&self, kid: cdk02::Id) -> Result<Option<MintKeySetInfo>> {
        let rid = RecordId::from_table_key(self.keys_table.clone(), kid.to_string());
        let info: Option<MintKeySetInfo> = self
            .db
            .query("SELECT VALUE info FROM $rid")
            .bind(("rid", rid))
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        Ok(info)
    }
    async fn keyset(&self, kid: cdk02::Id) -> Result<Option<cdk02::MintKeySet>> {
        let rid = RecordId::from_table_key(self.keys_table.clone(), kid.to_string());
        let keyset = self
            .entry(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?
            .map(|(_, keyset)| keyset);
        Ok(keyset)
    }
    async fn list_info(&self) -> Result<Vec<MintKeySetInfo>> {
        let infos: Vec<MintKeySetInfo> = self
            .db
            .query("SELECT VALUE info FROM type::table($table)")
            .bind(("table", self.keys_table.clone()))
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        Ok(infos)
    }
    async fn list_keyset(&self) -> Result<Vec<cdk02::MintKeySet>> {
        let response: Vec<KeysDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table)")
            .bind(("table", self.keys_table.clone()))
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        let sets = response
            .into_iter()
            .map(KeysetEntry::from)
            .map(|(_, keyset)| keyset)
            .collect();
        Ok(sets)
    }
    async fn update_info(&self, info: MintKeySetInfo) -> Result<()> {
        let kid = info.id;
        let rid = RecordId::from_table_key(self.keys_table.clone(), info.id.to_string());
        let entry: Option<KeysDBEntry> = self
            .db
            .query("UPDATE $rid SET info = $info RETURN BEFORE")
            .bind(("rid", rid))
            .bind(("info", info))
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        if entry.is_some() {
            Ok(())
        } else {
            Err(Error::UnknownKeyset(kid))
        }
    }
    async fn infos_for_expiration_date(&self, expire: TStamp) -> Result<Vec<MintKeySetInfo>> {
        let tstamp = expire.timestamp();
        let infos: Vec<MintKeySetInfo> = self
            .db
            .query("SELECT VALUE info FROM type::table($table) WHERE info.final_expiry > $tstamp ORDER BY info.final_expiry")
            .bind(("table", self.keys_table.clone()))
            .bind(("tstamp", tstamp))
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        Ok(infos)
    }

    async fn store_mintop(&self, mint_op: MintOperation) -> Result<()> {
        if self.info(mint_op.kid).await?.is_none() {
            return Err(Error::UnknownKeyset(mint_op.kid));
        }
        let entry = convert_to_mintopdbentry(mint_op, &self.mints_table);
        let _: Vec<MintOpDBEntry> = self
            .db
            .insert(&self.mints_table)
            .content(entry)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        Ok(())
    }

    async fn load_mintop(&self, uid: Uuid) -> Result<MintOperation> {
        let rid = RecordId::from_table_key(&self.mints_table, uid);
        let entry: Option<MintOpDBEntry> = self
            .db
            .select(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        let entry = entry.ok_or(Error::InvalidMintRequest(format!("unknown quote {uid}")))?;
        Ok(MintOperation::from(entry))
    }

    async fn list_mintops(&self, kid: cashu::Id) -> Result<Vec<MintOperation>> {
        let ops: Vec<MintOpDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE kid == $kid")
            .bind(("table", self.mints_table.clone()))
            .bind(("kid", kid))
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;

        let ops = ops.into_iter().map(MintOperation::from).collect();
        Ok(ops)
    }
    async fn update_mintop(&self, uid: Uuid, minted: cashu::Amount) -> Result<()> {
        let rid = RecordId::from_table_key(self.mints_table.clone(), uid);
        let before: Option<MintOpDBEntry> = self
            .db
            .query("UPDATE $rid SET minted = $minted RETURN BEFORE")
            .bind(("rid", rid))
            .bind(("minted", minted))
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        if let Some(before) = before {
            if before.minted < minted {
                tracing::error!("DBKeys::update_mint_operation with minted < mintop.minted");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SignatureDBEntry {
    pub amount: Amount,
    pub keyset_id: cdk02::Id,
    pub c: PublicKey,
    pub dleq: Option<cdk12::BlindSignatureDleq>,
}
impl std::convert::From<cdk00::BlindSignature> for SignatureDBEntry {
    fn from(sig: cdk00::BlindSignature) -> Self {
        Self {
            amount: sig.amount,
            keyset_id: sig.keyset_id,
            c: sig.c,
            dleq: sig.dleq,
        }
    }
}
impl std::convert::From<SignatureDBEntry> for cdk00::BlindSignature {
    fn from(entry: SignatureDBEntry) -> Self {
        Self {
            amount: entry.amount,
            keyset_id: entry.keyset_id,
            c: entry.c,
            dleq: entry.dleq,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct DBSignaturesConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub table: String,
}

#[derive(Debug, Clone)]
pub struct DBSignatures {
    db: Surreal<surrealdb::engine::any::Any>,
    table: String,
}

impl DBSignatures {
    pub async fn new(cfg: DBSignaturesConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self {
            db: db_connection,
            table: cfg.table,
        })
    }
}

#[async_trait]
impl SignaturesRepository for DBSignatures {
    async fn store(&self, y: cdk01::PublicKey, signature: cdk00::BlindSignature) -> Result<()> {
        let rid = RecordId::from_table_key(self.table.clone(), y.to_string());
        let entry = SignatureDBEntry::from(signature);
        let r: SurrealResult<Option<SignatureDBEntry>> = self.db.insert(rid).content(entry).await;
        match r {
            Err(SurrealError::Db(SurrealDBError::RecordExists { .. })) => {
                Err(Error::SignatureAlreadyExists(y))
            }
            Err(e) => Err(Error::SignaturesRepository(anyhow!(e))),
            Ok(..) => Ok(()),
        }
    }
    async fn load(&self, blind: &cdk00::BlindedMessage) -> Result<Option<cdk00::BlindSignature>> {
        let rid = RecordId::from_table_key(self.table.clone(), blind.blinded_secret.to_string());
        let entry: Option<SignatureDBEntry> = self
            .db
            .select(rid)
            .await
            .map_err(|e| Error::SignaturesRepository(anyhow!(e)))?;
        Ok(entry.map(cdk00::BlindSignature::from))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_wdc_utils::{keys::test_utils as keys_test, signatures::test_utils as signatures_test};

    async fn init_mem_db() -> DBKeys {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBKeys {
            db: sdb,
            keys_table: String::from("keys_test"),
            mints_table: String::from("mints_test"),
        }
    }

    #[tokio::test]
    async fn info() {
        let db = init_mem_db().await;
        let (info, keyset) = keys_test::generate_random_keyset();
        let dbkeys = convert_to_keysdbentry((info.clone(), keyset), &db.keys_table);
        let rid = RecordId::from_table_key(db.keys_table.clone(), info.id.to_string());
        let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();

        let rinfo = db.info(info.id).await.unwrap().unwrap();
        assert_eq!(rinfo, info);
    }

    #[tokio::test]
    async fn list_info() {
        let db = init_mem_db().await;
        {
            let (info, keyset) = keys_test::generate_random_keyset();
            let rid = RecordId::from_table_key(db.keys_table.clone(), info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), &db.keys_table);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        {
            let (info, keyset) = keys_test::generate_random_keyset();
            let rid = RecordId::from_table_key(db.keys_table.clone(), info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), &db.keys_table);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }

        let rinfos = db.list_info().await.unwrap();
        assert_eq!(rinfos.len(), 2);
    }

    #[tokio::test]
    async fn keyset() {
        let db = init_mem_db().await;
        let (info, keyset) = keys_test::generate_random_keyset();
        let rid = RecordId::from_table_key(db.keys_table.clone(), info.id.to_string());
        let dbkeys = convert_to_keysdbentry((info.clone(), keyset.clone()), &db.keys_table);
        let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();

        let rkeys = db.keyset(info.id).await.unwrap().unwrap();
        assert_eq!(rkeys, keyset);
    }

    #[tokio::test]
    async fn list_keyset() {
        let db = init_mem_db().await;
        {
            let (info, keyset) = keys_test::generate_random_keyset();
            let rid = RecordId::from_table_key(db.keys_table.clone(), info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), &db.keys_table);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        {
            let (info, keyset) = keys_test::generate_random_keyset();
            let rid = RecordId::from_table_key(db.keys_table.clone(), info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), &db.keys_table);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        let rkeys = db.list_keyset().await.unwrap();
        assert_eq!(rkeys.len(), 2);
    }

    #[tokio::test]
    async fn update_info() {
        let db = init_mem_db().await;
        let (mut info, keyset) = keys_test::generate_random_keyset();
        let rid = RecordId::from_table_key(db.keys_table.clone(), info.id.to_string());
        let dbkeys = convert_to_keysdbentry((info.clone(), keyset), &db.keys_table);
        let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        info.active = false;
        db.update_info(info.clone()).await.unwrap();
        let updated_info = db.info(info.id).await.unwrap().unwrap();
        assert!(!updated_info.active);
    }

    #[tokio::test]
    async fn update_info_kid_not_present() {
        let db = init_mem_db().await;
        let (mut info, _) = keys_test::generate_random_keyset();
        info.active = false;
        let res = db.update_info(info).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn store_mintop() {
        let db = init_mem_db().await;
        let keys = keys_test::generate_random_keyset();
        let kid = keys.0.id;
        db.store(keys).await.unwrap();
        let kp = keys_test::generate_random_keypair();
        let op = MintOperation{
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: Amount::ZERO,
            minted: Amount::ZERO,
        };
        db.store_mintop(op).await.unwrap();
    }

    #[tokio::test]
    async fn store_mintop_unknownkeyset() {
        let db = init_mem_db().await;
        let keys = keys_test::generate_random_keyset();
        let kid = keys.0.id;
        let kp = keys_test::generate_random_keypair();
        let op = MintOperation{
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: Amount::ZERO,
            minted: Amount::ZERO,
        };
        assert!(db.store_mintop(op).await.is_err());
    }

    #[tokio::test]
    async fn load_mintop() {
        let db = init_mem_db().await;
        let keys = keys_test::generate_random_keyset();
        let kid = keys.0.id;
        let kp = keys_test::generate_random_keypair();
        let op = MintOperation{
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: Amount::ZERO,
            minted: Amount::ZERO,
        };
        db.store(keys).await.unwrap();
        db.store_mintop(op.clone()).await.unwrap();
        let res = db.load_mintop(op.uid).await.unwrap();
        assert_eq!(res.kid , kid);
        assert_eq!(res.pub_key, kp.public_key().into());
    }

    #[tokio::test]
    async fn update_mintop() {
        let db = init_mem_db().await;
        let keys = keys_test::generate_random_keyset();
        let kid = keys.0.id;
        let kp = keys_test::generate_random_keypair();
        let op = MintOperation{
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: Amount::ZERO,
            minted: Amount::ZERO,
        };
        db.store(keys).await.unwrap();
        db.store_mintop(op.clone()).await.unwrap();
        db.update_mintop(op.uid, Amount::from(100u64)).await.unwrap();
        let res = db.load_mintop(op.uid).await.unwrap();
        assert_eq!(res.kid , kid);
        assert_eq!(res.minted, Amount::from(100u64));
    }

    #[tokio::test]
    async fn list_mintops() {
        let db = init_mem_db().await;
        let keys = keys_test::generate_random_keyset();
        let kid = keys.0.id;
        let kp = keys_test::generate_random_keypair();
        let op1 = MintOperation{
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: Amount::ZERO,
            minted: Amount::ZERO,
        };
        db.store(keys).await.unwrap();
        db.store_mintop(op1.clone()).await.unwrap();
        let op2 = MintOperation{
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: Amount::ZERO,
            minted: Amount::ZERO,
        };
        db.store_mintop(op2.clone()).await.unwrap();
        let res = db.list_mintops(kid).await.unwrap();
        assert_eq!(res.len(), 2);
        let rids: Vec<_>= res.iter().map(|op| op.uid).collect();
        assert!(rids.contains(&op1.uid));
        assert!(rids.contains(&op2.uid));
    }

    async fn init_mem_dbsignatures() -> DBSignatures {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBSignatures {
            db: sdb,
            table: String::from("test"),
        }
    }

    #[tokio::test]
    async fn dbsignatures_store() {
        let db = init_mem_dbsignatures().await;
        let (_, keyset) = keys_test::generate_random_keyset();
        let amounts = [Amount::from(8u64)];

        let y = keys_test::publics()[0];
        let signature = signatures_test::generate_signatures(&keyset, &amounts)[0].clone();

        db.store(y, signature).await.unwrap();
    }

    #[tokio::test]
    async fn dbsignatures_store_same_signature_twice() {
        let db = init_mem_dbsignatures().await;
        let (_, keyset) = keys_test::generate_random_keyset();
        let amounts = [Amount::from(8u64)];

        let y = keys_test::publics()[0];
        let signature = signatures_test::generate_signatures(&keyset, &amounts)[0].clone();

        db.store(y, signature.clone()).await.unwrap();
        let res = db.store(y, signature).await;
        assert!(matches!(res, Err(Error::SignatureAlreadyExists(..))));
    }
}
