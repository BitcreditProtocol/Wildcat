// ----- standard library imports
use std::collections::{BTreeMap, HashMap};
// ----- extra library imports
use anyhow::anyhow;
use async_trait::async_trait;
use bcr_common::{
    cashu::{
        self,
        nut01::{MintKeyPair, MintKeys},
    },
    cdk_common::mint::MintKeySetInfo,
};
use bcr_wdc_utils::{keys::KeysetEntry, surreal};
use bitcoin::bip32::DerivationPath;
use surrealdb::{
    engine::any::Any, error::Db as SurrealDBError, Error as SurrealError, RecordId,
    Result as SurrealResult, Surreal,
};
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence,
};

// ----- end imports

//////////////////////////////////////////////////////////////////////// Keys DB
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct KeysInfoDBEntry {
    kid: cashu::Id,
    unit: cashu::CurrencyUnit,
    active: bool,
    valid_from: u64,
    derivation_path: DerivationPath,
    derivation_path_index: Option<u32>,
    max_order: u8,
    denominations: Vec<u64>,
    input_fee_ppk: u64,
    final_expiry: Option<u64>,
}
impl std::convert::From<KeysInfoDBEntry> for MintKeySetInfo {
    fn from(info: KeysInfoDBEntry) -> Self {
        Self {
            id: info.kid,
            unit: info.unit,
            active: info.active,
            valid_from: info.valid_from,
            derivation_path: info.derivation_path,
            derivation_path_index: info.derivation_path_index,
            max_order: info.max_order,
            input_fee_ppk: info.input_fee_ppk,
            final_expiry: info.final_expiry,
            amounts: info.denominations,
        }
    }
}
impl std::convert::From<MintKeySetInfo> for KeysInfoDBEntry {
    fn from(info: MintKeySetInfo) -> Self {
        Self {
            kid: info.id,
            unit: info.unit,
            active: info.active,
            valid_from: info.valid_from,
            derivation_path: info.derivation_path,
            derivation_path_index: info.derivation_path_index,
            max_order: info.max_order,
            input_fee_ppk: info.input_fee_ppk,
            final_expiry: info.final_expiry,
            denominations: info.amounts,
        }
    }
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct KeysDBEntry {
    id: RecordId,
    info: KeysInfoDBEntry,
    keys: HashMap<String, MintKeyPair>,
}

fn convert_to_keysdbentry(entry: KeysetEntry, table: &str) -> KeysDBEntry {
    let (info, keyset) = entry;
    let id = RecordId::from_table_key(table, info.id.to_string());
    let mut serialized_keys = HashMap::new();
    let cashu::MintKeySet { mut keys, .. } = keyset;
    while let Some((amount, keypair)) = keys.pop_last() {
        // surrealDB does not accept map with keys of type anything but Strings
        // so we need to serialize the keys to strings...
        serialized_keys.insert(amount.to_string(), keypair);
    }
    KeysDBEntry {
        id,
        info: KeysInfoDBEntry::from(info),
        keys: serialized_keys,
    }
}
impl std::convert::From<KeysDBEntry> for KeysetEntry {
    fn from(dbk: KeysDBEntry) -> Self {
        let KeysDBEntry { info, keys, id: _ } = dbk;
        let info = MintKeySetInfo::from(info);
        let mut keysmap: BTreeMap<cashu::Amount, MintKeyPair> = BTreeMap::default();
        for (val, keypair) in keys.into_iter() {
            // ... and parse them back to the original type
            let uval = val.parse::<u64>().expect("Failed to parse amount");
            keysmap.insert(cashu::Amount::from(uval), keypair);
        }
        let keyset = cashu::MintKeySet {
            id: info.id,
            unit: info.unit.clone(),
            keys: MintKeys::new(keysmap),
            final_expiry: info.final_expiry,
        };
        (info, keyset)
    }
}

#[derive(Debug, Clone)]
pub struct DBKeys {
    db: Surreal<surrealdb::engine::any::Any>,
}

impl DBKeys {
    const TABLE: &'static str = "keys";
    pub async fn new(cfg: surreal::DBConnConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self { db: db_connection })
    }
}

#[async_trait]
impl persistence::KeysRepository for DBKeys {
    async fn store(&self, entry: KeysetEntry) -> Result<()> {
        let rid = RecordId::from_table_key(Self::TABLE, entry.0.id.to_string());
        let dbentry = convert_to_keysdbentry(entry, Self::TABLE);
        let _resp: Option<KeysDBEntry> = self
            .db
            .insert(rid)
            .content(dbentry)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        Ok(())
    }

    async fn info(&self, kid: cashu::Id) -> Result<Option<MintKeySetInfo>> {
        let rid = RecordId::from_table_key(Self::TABLE, kid.to_string());
        let info: Option<KeysInfoDBEntry> = self
            .db
            .query("SELECT VALUE info FROM $rid")
            .bind(("rid", rid))
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        let info = info.map(MintKeySetInfo::from);
        Ok(info)
    }

    async fn keyset(&self, kid: cashu::Id) -> Result<Option<cashu::MintKeySet>> {
        let rid = RecordId::from_table_key(Self::TABLE, kid.to_string());
        let entry: Option<KeysDBEntry> = self
            .db
            .select(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        let keyset = entry.map(KeysetEntry::from).map(|(_, keyset)| keyset);
        Ok(keyset)
    }

    async fn list_info(
        &self,
        unit: Option<cashu::CurrencyUnit>,
        min_exp_tstamp: Option<u64>,
        max_exp_tstamp: Option<u64>,
    ) -> Result<Vec<MintKeySetInfo>> {
        let mut statement = String::from("SELECT VALUE info FROM type::table($table)");
        let mut joiner = "WHERE";
        if unit.is_some() {
            statement.push_str(&format!(" {joiner} info.unit = $unit"));
            joiner = "AND";
        }
        if min_exp_tstamp.is_some() {
            statement.push_str(&format!(" {joiner} info.final_expiry >= $min"));
            joiner = "AND";
        }
        if max_exp_tstamp.is_some() {
            statement.push_str(&format!(" {joiner} info.final_expiry <= $max"));
        }
        let infos: Vec<KeysInfoDBEntry> = self
            .db
            .query(statement)
            .bind(("table", Self::TABLE))
            .bind(("unit", unit.map(|u| u.to_string())))
            .bind(("min", min_exp_tstamp))
            .bind(("max", max_exp_tstamp))
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        let infos = infos.into_iter().map(MintKeySetInfo::from).collect();
        Ok(infos)
    }

    async fn list_keyset(&self) -> Result<Vec<cashu::MintKeySet>> {
        let response: Vec<KeysDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table)")
            .bind(("table", Self::TABLE))
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
        let info = KeysInfoDBEntry::from(info);
        let kid = info.kid;
        let rid = RecordId::from_table_key(Self::TABLE, kid.to_string());
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
            Err(Error::KeysetNotFound(kid))
        }
    }

    async fn infos_for_expiration_date(&self, expire: u64) -> Result<Vec<MintKeySetInfo>> {
        let infos: Vec<KeysInfoDBEntry> = self
            .db
            // WARNING: https://github.com/surrealdb/surrealdb/issues/6405
            // .query("SELECT info FROM type::table($table) WHERE info.final_expiry > $tstamp ORDER BY info.final_expiry ASC")
            .query("SELECT VALUE info FROM type::table($table) WHERE info.final_expiry >= $expire")
            .bind(("table", Self::TABLE))
            .bind(("expire", expire))
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        let infos = infos.into_iter().map(MintKeySetInfo::from).collect();
        Ok(infos)
    }
}

////////////////////////////////////////////////////////////////// Signatures DB
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SignatureDBEntry {
    pub amount: cashu::Amount,
    pub keyset_id: cashu::Id,
    pub c: cashu::PublicKey,
    pub dleq: Option<cashu::BlindSignatureDleq>,
}
impl std::convert::From<cashu::BlindSignature> for SignatureDBEntry {
    fn from(sig: cashu::BlindSignature) -> Self {
        Self {
            amount: sig.amount,
            keyset_id: sig.keyset_id,
            c: sig.c,
            dleq: sig.dleq,
        }
    }
}
impl std::convert::From<SignatureDBEntry> for cashu::BlindSignature {
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
}

impl DBSignatures {
    const TABLE: &'static str = "signatures";

    pub async fn new(cfg: surreal::DBConnConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self { db: db_connection })
    }
}

#[async_trait]
impl persistence::SignaturesRepository for DBSignatures {
    async fn store(&self, y: cashu::PublicKey, signature: cashu::BlindSignature) -> Result<()> {
        let rid = RecordId::from_table_key(Self::TABLE, y.to_string());
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
    async fn load(&self, blind: &cashu::BlindedMessage) -> Result<Option<cashu::BlindSignature>> {
        let rid = RecordId::from_table_key(Self::TABLE, blind.blinded_secret.to_string());
        let entry: Option<SignatureDBEntry> = self
            .db
            .select(rid)
            .await
            .map_err(|e| Error::SignaturesRepository(anyhow!(e)))?;
        Ok(entry.map(cashu::BlindSignature::from))
    }
}

////////////////////////////////////////////////////////////////////// Proofs DB
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DBProof {
    id: RecordId,
    kid: cashu::Id,
    secret: cashu::secret::Secret,
    c: cashu::PublicKey,
    witness: Option<cashu::Witness>,
}
fn convert_to_db(proof: &cashu::Proof, table: &str) -> Result<DBProof> {
    let rid = proof_to_record_id(table, proof)?;
    let dbentry = DBProof {
        id: rid,
        kid: proof.keyset_id,
        secret: proof.secret.clone(),
        c: proof.c,
        witness: proof.witness.clone(),
    };
    Ok(dbentry)
}

#[derive(Debug, Clone)]
pub struct DBProofs {
    db: Surreal<surrealdb::engine::any::Any>,
}

impl DBProofs {
    const TABLE: &'static str = "proofs";
    pub async fn new(cfg: surreal::DBConnConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self { db: db_connection })
    }
}

#[async_trait]
impl persistence::ProofRepository for DBProofs {
    async fn insert(&self, tokens: &[cashu::Proof]) -> Result<()> {
        let mut entries: Vec<DBProof> = Vec::with_capacity(tokens.len());
        for tk in tokens {
            let db_entry = convert_to_db(tk, Self::TABLE)?;
            entries.push(db_entry);
        }
        let _: Vec<DBProof> = self
            .db
            .insert(())
            .content(entries)
            .await
            .map_err(|e| match e {
                surrealdb::Error::Db(surrealdb::error::Db::RecordExists { .. }) => {
                    Error::ProofsAlreadySpent
                }
                _ => Error::ProofRepository(anyhow!(e)),
            })?;
        Ok(())
    }

    async fn remove(&self, tokens: &[cashu::Proof]) -> Result<()> {
        for tk in tokens {
            let rid = proof_to_record_id(Self::TABLE, tk)?;
            let _p: Option<cashu::Proof> = self
                .db
                .delete(rid)
                .await
                .map_err(|e| Error::ProofRepository(anyhow!(e)))?;
        }
        Ok(())
    }

    async fn contains(&self, y: cashu::PublicKey) -> Result<Option<cashu::ProofState>> {
        let rid = y_to_record_id(Self::TABLE, y);
        let res: Option<DBProof> = self
            .db
            .select(rid)
            .await
            .map_err(|e| Error::ProofRepository(anyhow!(e)))?;
        if res.is_some() {
            let ret_v = cashu::ProofState {
                y,
                state: cashu::State::Spent,
                witness: None,
            };
            return Ok(Some(ret_v));
        }
        Ok(None)
    }
}

fn proof_to_record_id(main_table: &str, proof: &cashu::Proof) -> Result<RecordId> {
    let y = cashu::dhke::hash_to_curve(proof.secret.as_bytes()).map_err(Error::CdkDhke)?;
    Ok(y_to_record_id(main_table, y))
}
fn y_to_record_id(main_table: &str, y: cashu::PublicKey) -> RecordId {
    RecordId::from_table_key(main_table, y.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_wdc_utils::{keys::test_utils as keys_test, signatures::test_utils as signatures_test};
    use persistence::{KeysRepository, ProofRepository, SignaturesRepository};

    async fn init_keys_mem_db() -> DBKeys {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBKeys { db: sdb }
    }

    #[tokio::test]
    async fn info() {
        let db = init_keys_mem_db().await;
        let (info, keyset) = keys_test::generate_random_keyset();
        let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
        let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
        let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();

        let rinfo = db.info(info.id).await.unwrap().unwrap();
        assert_eq!(rinfo, info);
    }

    #[tokio::test]
    async fn list_info() {
        let db = init_keys_mem_db().await;
        {
            let (info, keyset) = keys_test::generate_random_keyset();
            let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        {
            let (info, keyset) = keys_test::generate_random_keyset();
            let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }

        let rinfos = db.list_info(None, None, None).await.unwrap();
        assert_eq!(rinfos.len(), 2);
    }

    #[tokio::test]
    async fn list_info_with_unit() {
        let db = init_keys_mem_db().await;
        {
            let (mut info, keyset) = keys_test::generate_random_keyset();
            info.unit = cashu::CurrencyUnit::Sat;
            info.final_expiry = Some(10);
            let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        {
            let (mut info, keyset) = keys_test::generate_random_keyset();
            info.unit = cashu::CurrencyUnit::Usd;
            let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }

        let rinfos = db
            .list_info(Some(cashu::CurrencyUnit::Sat), None, None)
            .await
            .unwrap();
        assert_eq!(rinfos.len(), 1);
        assert_eq!(rinfos[0].unit, cashu::CurrencyUnit::Sat);
    }

    #[tokio::test]
    async fn list_info_with_min_expiration() {
        let db = init_keys_mem_db().await;
        {
            let (mut info, keyset) = keys_test::generate_random_keyset();
            info.final_expiry = Some(10);
            let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        {
            let (mut info, keyset) = keys_test::generate_random_keyset();
            info.final_expiry = Some(20);
            let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }

        let rinfos = db.list_info(None, None, None).await.unwrap();
        assert_eq!(rinfos.len(), 2);
        let rinfos = db.list_info(None, Some(15), None).await.unwrap();
        assert_eq!(rinfos.len(), 1);
        assert_eq!(rinfos[0].final_expiry, Some(20));
    }

    #[tokio::test]
    async fn list_info_with_max_expiration() {
        let db = init_keys_mem_db().await;
        {
            let (mut info, keyset) = keys_test::generate_random_keyset();
            info.final_expiry = Some(10);
            let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        {
            let (mut info, keyset) = keys_test::generate_random_keyset();
            info.final_expiry = Some(20);
            let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }

        let rinfos = db.list_info(None, None, None).await.unwrap();
        assert_eq!(rinfos.len(), 2);
        let rinfos = db.list_info(None, None, Some(15)).await.unwrap();
        assert_eq!(rinfos.len(), 1);
        assert_eq!(rinfos[0].final_expiry, Some(10));
    }

    #[tokio::test]
    async fn keyset() {
        let db = init_keys_mem_db().await;
        let (info, keyset) = keys_test::generate_random_keyset();
        let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
        let dbkeys = convert_to_keysdbentry((info.clone(), keyset.clone()), DBKeys::TABLE);
        let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();

        let rkeys = db.keyset(info.id).await.unwrap().unwrap();
        assert_eq!(rkeys, keyset);
    }

    #[tokio::test]
    async fn list_keyset() {
        let db = init_keys_mem_db().await;
        {
            let (info, keyset) = keys_test::generate_random_keyset();
            let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        {
            let (info, keyset) = keys_test::generate_random_keyset();
            let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        let rkeys = db.list_keyset().await.unwrap();
        assert_eq!(rkeys.len(), 2);
    }

    #[tokio::test]
    async fn update_info() {
        let db = init_keys_mem_db().await;
        let (mut info, keyset) = keys_test::generate_random_keyset();
        let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
        let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
        let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        info.active = false;
        db.update_info(info.clone()).await.unwrap();
        let updated_info = db.info(info.id).await.unwrap().unwrap();
        assert!(!updated_info.active);
    }

    #[tokio::test]
    async fn update_info_kid_not_present() {
        let db = init_keys_mem_db().await;
        let (info, _) = keys_test::generate_random_keyset();
        let res = db.update_info(info).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    #[ignore = "SurrealDB issue #6405"]
    async fn infos_for_expiration_date() {
        let db = init_keys_mem_db().await;
        let mut keys0 = keys_test::generate_random_keyset();
        keys0.0.final_expiry = Some(30);
        keys0.1.final_expiry = keys0.0.final_expiry;
        db.store(keys0).await.unwrap();
        let mut keys1 = keys_test::generate_random_keyset();
        keys1.0.final_expiry = Some(10);
        keys1.1.final_expiry = keys1.0.final_expiry;
        db.store(keys1).await.unwrap();
        let res = db.infos_for_expiration_date(10).await.unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].final_expiry, Some(10));
        assert_eq!(res[1].final_expiry, Some(30));
        let res = db.infos_for_expiration_date(20).await.unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].final_expiry, Some(30));
    }

    async fn init_mem_dbsignatures() -> DBSignatures {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBSignatures { db: sdb }
    }

    #[tokio::test]
    async fn dbsignatures_store() {
        let db = init_mem_dbsignatures().await;
        let (_, keyset) = keys_test::generate_random_keyset();
        let amounts = [cashu::Amount::from(8u64)];

        let y = keys_test::publics()[0];
        let signature = signatures_test::generate_signatures(&keyset, &amounts)[0].clone();

        db.store(y, signature).await.unwrap();
    }

    #[tokio::test]
    async fn dbsignatures_store_same_signature_twice() {
        let db = init_mem_dbsignatures().await;
        let (_, keyset) = keys_test::generate_random_keyset();
        let amounts = [cashu::Amount::from(8u64)];

        let y = keys_test::publics()[0];
        let signature = signatures_test::generate_signatures(&keyset, &amounts)[0].clone();

        db.store(y, signature.clone()).await.unwrap();
        let res = db.store(y, signature).await;
        assert!(matches!(res, Err(Error::SignatureAlreadyExists(..))));
    }

    async fn init_proofs_mem_db() -> DBProofs {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBProofs { db: sdb }
    }

    #[tokio::test]
    async fn test_insert() {
        let db = init_proofs_mem_db().await;
        let (_, keyset) = keys_test::generate_keyset();
        let proofs = signatures_test::generate_proofs(
            &keyset,
            &[cashu::Amount::from(16_u64), cashu::Amount::from(8_u64)],
        );
        db.insert(&proofs).await.unwrap();

        let rid = proof_to_record_id(DBProofs::TABLE, &proofs[0]).expect("Failed to get record id");
        let res: Option<DBProof> = db.db.select(rid).await.unwrap();
        assert!(res.is_some());
        assert_eq!(res.unwrap().secret, proofs[0].secret);

        let rid = proof_to_record_id(DBProofs::TABLE, &proofs[1]).expect("Failed to get record id");
        let res: Option<DBProof> = db.db.select(rid).await.unwrap();
        assert!(res.is_some());
        assert_eq!(res.unwrap().secret, proofs[1].secret);
    }

    #[tokio::test]
    async fn test_insert_double_spent_all() {
        let db = init_proofs_mem_db().await;
        let (_, keyset) = keys_test::generate_keyset();
        let proofs = signatures_test::generate_proofs(
            &keyset,
            &[cashu::Amount::from(16_u64), cashu::Amount::from(8_u64)],
        );
        db.insert(&proofs).await.unwrap();

        let res = db.insert(&proofs).await;
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::ProofsAlreadySpent));
    }

    #[tokio::test]
    async fn test_insert_double_spent_partial() {
        let db = init_proofs_mem_db().await;
        let (_, keyset) = keys_test::generate_keyset();
        let proofs = signatures_test::generate_proofs(
            &keyset,
            &[
                cashu::Amount::from(16_u64),
                cashu::Amount::from(8_u64),
                cashu::Amount::from(4_u64),
            ],
        );
        db.insert(&proofs[0..2]).await.unwrap();

        let res = db.insert(&proofs[1..]).await;
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::ProofsAlreadySpent));
    }

    #[tokio::test]
    async fn test_insert_double_spent_partial_still_valid() {
        let db = init_proofs_mem_db().await;
        let (_, keyset) = keys_test::generate_keyset();
        let proofs = signatures_test::generate_proofs(
            &keyset,
            &[
                cashu::Amount::from(16_u64),
                cashu::Amount::from(8_u64),
                cashu::Amount::from(4_u64),
            ],
        );
        db.insert(&proofs[0..2]).await.unwrap();

        let res = db.insert(&proofs[1..]).await;
        assert!(res.is_err());
        db.insert(&proofs[2..]).await.unwrap();
    }
}
