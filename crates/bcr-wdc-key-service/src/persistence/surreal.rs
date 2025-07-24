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
// ----- local imports
use crate::error::{Error, Result};
use crate::service::{KeysRepository, MintCondition, QuoteKeysRepository, SignaturesRepository};

// ----- end imports

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KeysDBEntry {
    info: MintKeySetInfo,
    // unpacking MintKeySet because surrealdb doesn't support BTreeMap<K,V> where K is not a String
    unit: cdk00::CurrencyUnit,
    // surrealdb supports only strings as key type
    keys: HashMap<String, cdk01::MintKeyPair>,
    final_expiry: Option<u64>,

    condition: MintCondition,
}

fn convert_from(ke: KeysetEntry, condition: MintCondition) -> KeysDBEntry {
    let (info, keyset) = ke;
    let mut serialized_keys = HashMap::new();
    let cdk02::MintKeySet { unit, mut keys, .. } = keyset;
    while let Some((amount, keypair)) = keys.pop_last() {
        // surrealDB does not accept map with keys of type anything but Strings
        // so we need to serialize the keys to strings...
        serialized_keys.insert(amount.to_string(), keypair);
    }
    KeysDBEntry {
        info,
        unit,
        keys: serialized_keys,
        final_expiry: keyset.final_expiry,
        condition,
    }
}

fn convert_to(dbk: KeysDBEntry) -> (KeysetEntry, MintCondition) {
    let KeysDBEntry {
        info,
        unit,
        keys,
        final_expiry,
        condition,
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
    ((info, keyset), condition)
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub table: String,
}

#[derive(Debug, Clone)]
struct DB {
    db: Surreal<surrealdb::engine::any::Any>,
    table: String,
}

impl DB {
    async fn new(cfg: ConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self {
            db: db_connection,
            table: cfg.table,
        })
    }

    async fn store(
        &self,
        rid: RecordId,
        keys: KeysetEntry,
        condition: MintCondition,
    ) -> SurrealResult<()> {
        let dbkeys = convert_from(keys, condition);
        let _resp: Option<KeysDBEntry> = self.db.insert(rid).content(dbkeys).await?;
        Ok(())
    }

    async fn condition(&self, rid: RecordId) -> SurrealResult<Option<MintCondition>> {
        let result: Option<MintCondition> = self
            .db
            .query("SELECT VALUE condition FROM $rid")
            .bind(("rid", rid))
            .await?
            .take(0)?;
        Ok(result)
    }

    async fn mark_as_minted(&self, rid: RecordId) -> SurrealResult<Option<MintCondition>> {
        let result: Option<KeysDBEntry> = self
            .db
            .query("UPDATE $rid SET condition.is_minted = true RETURN BEFORE")
            .bind(("rid", rid))
            .await?
            .take(0)?;
        let before_condition = result.map(|KeysDBEntry { condition, .. }| condition);
        Ok(before_condition)
    }

    async fn info(&self, rid: RecordId) -> SurrealResult<Option<MintKeySetInfo>> {
        let info: Option<MintKeySetInfo> = self
            .db
            .query("SELECT VALUE info FROM $rid")
            .bind(("rid", rid))
            .await?
            .take(0)?;
        Ok(info)
    }

    async fn update_info(
        &self,
        rid: RecordId,
        info: MintKeySetInfo,
    ) -> SurrealResult<Option<MintKeySetInfo>> {
        let entry: Option<KeysDBEntry> = self
            .db
            .query("UPDATE $rid SET info = $info")
            .bind(("rid", rid))
            .bind(("info", info))
            .await?
            .take(0)?;
        let info = entry.map(|KeysDBEntry { info, .. }| info);
        Ok(info)
    }

    async fn list_info(&self) -> SurrealResult<Vec<MintKeySetInfo>> {
        let infos: Vec<MintKeySetInfo> = self
            .db
            .query("SELECT VALUE info FROM type::table($table)")
            .bind(("table", self.table.clone()))
            .await?
            .take(0)?;
        Ok(infos)
    }

    async fn entry(&self, rid: RecordId) -> SurrealResult<Option<KeysetEntry>> {
        let response: Option<KeysDBEntry> = self.db.select(rid).await?;
        let Some(keysdbentry) = response else {
            return Ok(None);
        };
        let (entry, _) = convert_to(keysdbentry);
        Ok(Some(entry))
    }

    async fn keyset(&self, rid: RecordId) -> SurrealResult<Option<cdk02::MintKeySet>> {
        let keyset = self.entry(rid).await?.map(|(_, keyset)| keyset);
        Ok(keyset)
    }

    async fn list_keyset(&self) -> SurrealResult<Vec<cdk02::MintKeySet>> {
        let response: Vec<KeysDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table)")
            .bind(("table", self.table.clone()))
            .await?
            .take(0)?;
        let sets = response
            .into_iter()
            .map(convert_to)
            .map(|((_, keyset), _)| keyset)
            .collect();
        Ok(sets)
    }
}

#[derive(Debug, Clone)]
pub struct DBKeys(DB);

impl DBKeys {
    pub async fn new(cfg: ConnectionConfig) -> SurrealResult<Self> {
        Ok(Self(DB::new(cfg).await?))
    }
}

#[async_trait]
impl KeysRepository for DBKeys {
    async fn condition(&self, kid: &cdk02::Id) -> Result<Option<MintCondition>> {
        let rid = RecordId::from_table_key(self.0.table.clone(), kid.to_string());
        self.0
            .condition(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }
    async fn mark_as_minted(&self, kid: &cdk02::Id) -> Result<()> {
        let rid = RecordId::from_table_key(self.0.table.clone(), kid.to_string());
        let before = self
            .0
            .mark_as_minted(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?
            .ok_or(Error::UnknownKeyset(*kid))?;
        if before.is_minted {
            return Err(Error::InvalidMintRequest(format!(
                "Keyset {kid} already minted"
            )));
        }
        Ok(())
    }

    async fn info(&self, kid: &cdk02::Id) -> Result<Option<MintKeySetInfo>> {
        let rid = RecordId::from_table_key(self.0.table.clone(), kid.to_string());
        self.0
            .info(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }
    async fn update_info(&self, kid: &cdk02::Id, info: MintKeySetInfo) -> Result<()> {
        let rid = RecordId::from_table_key(self.0.table.clone(), kid.to_string());
        let opt_info = self
            .0
            .update_info(rid, info)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        opt_info.ok_or(Error::UnknownKeyset(*kid))?;
        Ok(())
    }
    async fn list_info(&self) -> Result<Vec<MintKeySetInfo>> {
        self.0
            .list_info()
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }
    async fn keyset(&self, kid: &cdk02::Id) -> Result<Option<cdk02::MintKeySet>> {
        let rid = RecordId::from_table_key(self.0.table.clone(), kid.to_string());
        self.0
            .keyset(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }

    async fn list_keyset(&self) -> Result<Vec<cdk02::MintKeySet>> {
        self.0
            .list_keyset()
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }

    async fn store(&self, entry: KeysetEntry, condition: MintCondition) -> Result<()> {
        let rid = RecordId::from_table_key(self.0.table.clone(), entry.0.id.to_string());
        self.0
            .store(rid, entry, condition)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }
}

#[derive(Debug, Clone)]
pub struct DBQuoteKeys(DB);

impl DBQuoteKeys {
    pub async fn new(cfg: ConnectionConfig) -> SurrealResult<Self> {
        Ok(Self(DB::new(cfg).await?))
    }
}

#[async_trait]
impl QuoteKeysRepository for DBQuoteKeys {
    async fn entry(&self, qid: &uuid::Uuid) -> Result<Option<KeysetEntry>> {
        let rid = RecordId::from_table_key(self.0.table.clone(), *qid);
        self.0
            .entry(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }

    async fn info(&self, qid: &uuid::Uuid) -> Result<Option<MintKeySetInfo>> {
        let rid = RecordId::from_table_key(self.0.table.clone(), *qid);
        self.0
            .info(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }

    async fn condition(&self, qid: &uuid::Uuid) -> Result<Option<MintCondition>> {
        let rid = RecordId::from_table_key(self.0.table.clone(), *qid);
        self.0
            .condition(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }

    async fn keyset(&self, qid: &uuid::Uuid) -> Result<Option<cdk02::MintKeySet>> {
        let rid = RecordId::from_table_key(self.0.table.clone(), *qid);
        self.0
            .keyset(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
    }

    async fn store(
        &self,
        qid: &uuid::Uuid,
        entry: KeysetEntry,
        condition: MintCondition,
    ) -> Result<()> {
        let rid = RecordId::from_table_key(self.0.table.clone(), *qid);
        self.0
            .store(rid, entry, condition)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))
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

#[derive(Debug, Clone)]
pub struct DBSignatures {
    db: Surreal<surrealdb::engine::any::Any>,
    table: String,
}

impl DBSignatures {
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

    async fn init_mem_db() -> DB {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DB {
            db: sdb,
            table: String::from("test"),
        }
    }

    #[tokio::test]
    async fn condition() {
        let db = init_mem_db().await;
        let (info, keyset) = keys_test::generate_random_keyset();
        let pk = keys_test::publics()[0];
        let mint_condition = MintCondition {
            target: Amount::from(199),
            pub_key: pk,
            is_minted: false,
        };
        let rid = RecordId::from_table_key(db.table.clone(), info.id.to_string());
        let dbkeys = convert_from((info, keyset), mint_condition.clone());
        let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();

        let rcondition = db.condition(rid).await.unwrap().unwrap();
        assert_eq!(rcondition, mint_condition);
    }

    #[tokio::test]
    async fn info() {
        let db = init_mem_db().await;
        let (info, keyset) = keys_test::generate_random_keyset();
        let pk = keys_test::publics()[0];
        let mint_condition = MintCondition {
            target: Amount::from(199),
            pub_key: pk,
            is_minted: false,
        };
        let rid = RecordId::from_table_key(db.table.clone(), info.id.to_string());
        let dbkeys = convert_from((info.clone(), keyset), mint_condition);
        let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();

        let rinfo = db.info(rid).await.unwrap().unwrap();
        assert_eq!(rinfo, info);
    }

    #[tokio::test]
    async fn list_info() {
        let db = init_mem_db().await;
        {
            let (info, keyset) = keys_test::generate_random_keyset();
            let pk = keys_test::publics()[1];
            let mint_condition = MintCondition {
                target: Amount::from(99),
                pub_key: pk,
                is_minted: false,
            };
            let rid = RecordId::from_table_key(db.table.clone(), info.id.to_string());
            let dbkeys = convert_from((info.clone(), keyset), mint_condition);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        {
            let (info, keyset) = keys_test::generate_random_keyset();
            let pk = keys_test::publics()[0];
            let mint_condition = MintCondition {
                target: Amount::from(199),
                pub_key: pk,
                is_minted: false,
            };
            let rid = RecordId::from_table_key(db.table.clone(), info.id.to_string());
            let dbkeys = convert_from((info.clone(), keyset), mint_condition);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }

        let rinfos = db.list_info().await.unwrap();
        assert_eq!(rinfos.len(), 2);
    }

    #[tokio::test]
    async fn keyset() {
        let db = init_mem_db().await;
        let (info, keyset) = keys_test::generate_random_keyset();
        let pk = keys_test::publics()[0];
        let mint_condition = MintCondition {
            target: Amount::from(199),
            pub_key: pk,
            is_minted: false,
        };
        let rid = RecordId::from_table_key(db.table.clone(), info.id.to_string());
        let dbkeys = convert_from((info, keyset.clone()), mint_condition);
        let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();

        let rkeys = db.keyset(rid).await.unwrap().unwrap();
        assert_eq!(rkeys, keyset);
    }

    #[tokio::test]
    async fn list_keyset() {
        let db = init_mem_db().await;
        {
            let (info, keyset) = keys_test::generate_random_keyset();
            let pk = keys_test::publics()[1];
            let mint_condition = MintCondition {
                target: Amount::from(99),
                pub_key: pk,
                is_minted: false,
            };
            let rid = RecordId::from_table_key(db.table.clone(), info.id.to_string());
            let dbkeys = convert_from((info.clone(), keyset), mint_condition);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        {
            let (info, keyset) = keys_test::generate_random_keyset();
            let pk = keys_test::publics()[0];
            let mint_condition = MintCondition {
                target: Amount::from(199),
                pub_key: pk,
                is_minted: false,
            };
            let rid = RecordId::from_table_key(db.table.clone(), info.id.to_string());
            let dbkeys = convert_from((info.clone(), keyset), mint_condition);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }

        let rkeys = db.list_keyset().await.unwrap();
        assert_eq!(rkeys.len(), 2);
    }

    #[tokio::test]
    async fn mark_as_minted_ok() {
        let db = init_mem_db().await;
        let (info, keyset) = keys_test::generate_random_keyset();
        let pk = keys_test::publics()[0];
        let mint_condition = MintCondition {
            target: Amount::from(199),
            pub_key: pk,
            is_minted: false,
        };
        let rid = RecordId::from_table_key(db.table.clone(), info.id.to_string());
        let dbkeys = convert_from((info, keyset.clone()), mint_condition);
        let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();

        let result = db.mark_as_minted(rid).await.unwrap().unwrap();
        assert!(!result.is_minted);
    }

    #[tokio::test]
    async fn mark_as_minted_ko() {
        let db = init_mem_db().await;
        let (info, keyset) = keys_test::generate_random_keyset();
        let pk = keys_test::publics()[0];
        let mint_condition = MintCondition {
            target: Amount::from(199),
            pub_key: pk,
            is_minted: true,
        };
        let kid = info.id;
        let rid = RecordId::from_table_key(db.table.clone(), info.id.to_string());
        let dbkeys = convert_from((info, keyset), mint_condition);
        let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();

        let dbk = DBKeys(db);
        let result = dbk.mark_as_minted(&kid).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn update_info() {
        let db = init_mem_db().await;
        let (mut info, keyset) = keys_test::generate_random_keyset();
        let pk = keys_test::publics()[0];
        let mint_condition = MintCondition {
            target: Amount::from(199),
            pub_key: pk,
            is_minted: true,
        };
        let kid = info.id;
        let rid = RecordId::from_table_key(db.table.clone(), info.id.to_string());
        let dbkeys = convert_from((info.clone(), keyset), mint_condition);
        let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();

        info.active = false;

        let dbk = DBKeys(db);
        dbk.update_info(&kid, info).await.unwrap();
        let updated_info = dbk.info(&kid).await.unwrap().unwrap();
        assert!(!updated_info.active);
    }

    #[tokio::test]
    async fn update_info_kid_not_present() {
        let db = init_mem_db().await;
        let (mut info, _) = keys_test::generate_random_keyset();
        let kid = info.id;

        info.active = false;

        let dbk = DBKeys(db);
        let res = dbk.update_info(&kid, info).await;
        assert!(res.is_err());
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
