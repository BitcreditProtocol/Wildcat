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
    core::BillId,
};
use bcr_wdc_utils::{keys::KeysetEntry, surreal};
use bitcoin::bip32::DerivationPath;
use surrealdb::{
    engine::any::Any, error::Db as SurrealDBError, Error as SurrealError, RecordId,
    Result as SurrealResult, Surreal,
};
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    keys::service::MintOperation,
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
    async fn list_info(&self) -> Result<Vec<MintKeySetInfo>> {
        let infos: Vec<KeysInfoDBEntry> = self
            .db
            .query("SELECT VALUE info FROM type::table($table)")
            .bind(("table", Self::TABLE))
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

////////////////////////////////////////////////////////////////////// MintOp DB
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MintOpDBEntry {
    id: RecordId,
    kid: cashu::Id,
    pub_key: cashu::PublicKey,
    target: cashu::Amount,
    minted: cashu::Amount,
    bill_id: BillId,
}

fn convert_to_mintopdbentry(entry: MintOperation, table: &str) -> MintOpDBEntry {
    let MintOperation {
        uid,
        kid,
        pub_key,
        target,
        minted,
        bill_id,
    } = entry;
    let id = RecordId::from_table_key(table, uid);
    MintOpDBEntry {
        id,
        kid,
        pub_key,
        target,
        minted,
        bill_id,
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
            bill_id: entry.bill_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DBMintOps {
    db: Surreal<surrealdb::engine::any::Any>,
}

impl DBMintOps {
    const TABLE: &'static str = "mints";
    pub async fn new(cfg: surreal::DBConnConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self { db: db_connection })
    }
}

#[async_trait]
impl persistence::MintOpRepository for DBMintOps {
    async fn store(&self, mint_op: MintOperation) -> Result<()> {
        let uid = mint_op.uid;
        let entry = convert_to_mintopdbentry(mint_op, Self::TABLE);
        let res: SurrealResult<Option<MintOpDBEntry>> =
            self.db.insert(&entry.id).content(entry).await;
        match res {
            Ok(..) => Ok(()),
            Err(SurrealError::Db(SurrealDBError::RecordExists { .. })) => {
                Err(Error::MintOpAlreadyExist(uid))
            }
            Err(e) => Err(Error::KeysRepository(anyhow!(e))),
        }
    }

    async fn load(&self, uid: Uuid) -> Result<MintOperation> {
        let rid = RecordId::from_table_key(Self::TABLE, uid);
        let res: SurrealResult<Option<MintOpDBEntry>> = self.db.select(rid.clone()).await;
        match res {
            Ok(Some(entry)) => Ok(MintOperation::from(entry)),
            Ok(None) => Err(Error::MintOpNotFound(uid)),
            Err(e) => Err(Error::KeysRepository(anyhow!(e))),
        }
    }

    async fn list(&self, kid: cashu::Id) -> Result<Vec<MintOperation>> {
        let ops: Vec<MintOpDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE kid == $kid")
            .bind(("table", Self::TABLE))
            .bind(("kid", kid))
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;

        let ops = ops.into_iter().map(MintOperation::from).collect();
        Ok(ops)
    }
    async fn update(&self, uid: Uuid, old: cashu::Amount, new: cashu::Amount) -> Result<()> {
        let rid = RecordId::from_table_key(Self::TABLE, uid);
        let before: Option<MintOpDBEntry> = self
            .db
            .query("UPDATE $rid SET minted = $new WHERE minted == $old RETURN BEFORE")
            .bind(("rid", rid))
            .bind(("new", new))
            .bind(("old", old))
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        let Some(before) = before else {
            return Err(Error::InvalidMintRequest(format!(
                "No mint operation with uid {uid} and minted amount {old} found"
            )));
        };
        assert_eq!(before.minted, old, "Minted amount did not match");
        Ok(())
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
    use persistence::{KeysRepository, MintOpRepository, ProofRepository, SignaturesRepository};

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

        let rinfos = db.list_info().await.unwrap();
        assert_eq!(rinfos.len(), 2);
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

    async fn init_mintops_mem_db() -> DBMintOps {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBMintOps { db: sdb }
    }

    #[tokio::test]
    async fn store_mintop() {
        let db = init_mintops_mem_db().await;
        let keys = keys_test::generate_random_keyset();
        let kid = keys.0.id;
        let kp = keys_test::generate_random_keypair();
        let op = MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.store(op).await.unwrap();
    }
    #[tokio::test]
    async fn store_mintop_twice() {
        let db = init_mintops_mem_db().await;
        let keys = keys_test::generate_random_keyset();
        let kid = keys.0.id;
        let kp = keys_test::generate_random_keypair();
        let op = MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.store(op.clone()).await.unwrap();
        let res = db.store(op).await;
        assert!(matches!(res, Err(Error::MintOpAlreadyExist(_))));
    }

    #[tokio::test]
    async fn load_mintop() {
        let db = init_mintops_mem_db().await;
        let keys = keys_test::generate_random_keyset();
        let kid = keys.0.id;
        let kp = keys_test::generate_random_keypair();
        let op = MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.store(op.clone()).await.unwrap();
        let res = db.load(op.uid).await.unwrap();
        assert_eq!(res.kid, kid);
        assert_eq!(res.pub_key, kp.public_key().into());
    }

    #[tokio::test]
    async fn update_mintop() {
        let db = init_mintops_mem_db().await;
        let keys = keys_test::generate_random_keyset();
        let kid = keys.0.id;
        let kp = keys_test::generate_random_keypair();
        let op = MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.store(op.clone()).await.unwrap();
        db.update(op.uid, cashu::Amount::ZERO, cashu::Amount::from(100u64))
            .await
            .unwrap();
        let res = db.load(op.uid).await.unwrap();
        assert_eq!(res.kid, kid);
        assert_eq!(res.minted, cashu::Amount::from(100u64));
    }

    #[tokio::test]
    async fn list_mintops() {
        let db = init_mintops_mem_db().await;
        let keys = keys_test::generate_random_keyset();
        let kid = keys.0.id;
        let kp = keys_test::generate_random_keypair();
        let op1 = MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.store(op1.clone()).await.unwrap();
        let op2 = MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.store(op2.clone()).await.unwrap();
        let res = db.list(kid).await.unwrap();
        assert_eq!(res.len(), 2);
        let rids: Vec<_> = res.iter().map(|op| op.uid).collect();
        assert!(rids.contains(&op1.uid));
        assert!(rids.contains(&op2.uid));
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
