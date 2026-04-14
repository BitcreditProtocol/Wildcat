// ----- standard library imports
use std::{
    collections::{BTreeMap, HashMap},
    str::FromStr,
};
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
use bitcoin::{bip32::DerivationPath, secp256k1::schnorr};
use surrealdb::{
    engine::any::Any, error::Db as SurrealDBError, Error as SurrealError, RecordId,
    Result as SurrealResult, Surreal,
};
// ----- local imports
use crate::{
    error::{Error, Result},
    persistence, TStamp,
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
            Err(Error::ResourceNotFound(format!("keyset {}", kid)))
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
                Err(Error::Conflict(format!("signature already exists: {y}")))
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
                    Error::InvalidInput(String::from("proofs already spent"))
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
    let y = cashu::dhke::hash_to_curve(proof.secret.as_bytes())?;
    Ok(y_to_record_id(main_table, y))
}
fn y_to_record_id(main_table: &str, y: cashu::PublicKey) -> RecordId {
    RecordId::from_table_key(main_table, y.to_string())
}

////////////////////////////////////////////////////////////////////// Commitments DB
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CommitmentDBEntry {
    id: RecordId,
    inputs: Vec<cashu::PublicKey>,
    outputs: Vec<cashu::PublicKey>,
    expiration: TStamp,
    wallet_key: cashu::PublicKey,
}

#[derive(Debug, Clone)]
pub struct DBCommitments {
    db: Surreal<Any>,
}

impl DBCommitments {
    const TABLE: &'static str = "commitments";

    pub async fn new(cfg: surreal::DBConnConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(cfg.connection).await?;
        db_connection.use_ns(cfg.namespace).await?;
        db_connection.use_db(cfg.database).await?;
        Ok(Self { db: db_connection })
    }
}

#[async_trait]
impl persistence::CommitmentRepository for DBCommitments {
    async fn clean_expired(&self, now: TStamp) -> Result<()> {
        self.db
            .query("DELETE FROM type::table($table) WHERE expiration < $now")
            .bind(("table", Self::TABLE))
            .bind(("now", now))
            .await
            .map_err(|e| Error::CommitmentRepository(anyhow!("SurrealDB error: {}", e)))?;
        Ok(())
    }

    async fn contains_inputs(&self, ys: &[cashu::PublicKey]) -> Result<bool> {
        let commitment: Option<CommitmentDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE array::is_empty(array::intersect(inputs, $ys)) = false LIMIT 1")
            .bind(("table", Self::TABLE))
            .bind(("ys", ys.to_vec()))
            .await
            .map_err(|e| Error::CommitmentRepository(anyhow!("SurrealDB error: {}", e)))?
            .take(0)
            .map_err(|e| Error::CommitmentRepository(anyhow!("SurrealDB error: {}", e)))?;
        Ok(commitment.is_some())
    }

    async fn contains_outputs(&self, secrets: &[cashu::PublicKey]) -> Result<bool> {
        let commitment: Option<CommitmentDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE array::is_empty(array::intersect(outputs, $secrets)) = false LIMIT 1")
            .bind(("table", Self::TABLE))
            .bind(("secrets", secrets.to_vec()))
            .await
            .map_err(|e| Error::CommitmentRepository(anyhow!("SurrealDB error: {}", e)))?
            .take(0)
            .map_err(|e| Error::CommitmentRepository(anyhow!("SurrealDB error: {}", e)))?;
        Ok(commitment.is_some())
    }

    async fn store(
        &self,
        inputs: Vec<cashu::PublicKey>,
        outputs: Vec<cashu::PublicKey>,
        expiration: TStamp,
        wallet_key: cashu::PublicKey,
        signature: schnorr::Signature,
    ) -> Result<()> {
        let rid = RecordId::from_table_key(Self::TABLE, signature.to_string());
        let entry = CommitmentDBEntry {
            id: rid.clone(),
            inputs,
            outputs,
            expiration,
            wallet_key,
        };
        let _: Option<CommitmentDBEntry> =
            self.db.insert(rid).content(entry).await.map_err(|e| {
                Error::CommitmentRepository(anyhow!(
                    "SurrealDB error while storing commitment: {}",
                    e
                ))
            })?;
        Ok(())
    }

    async fn load(
        &self,
        inputs: &[cashu::PublicKey],
        outputs: &[cashu::PublicKey],
    ) -> Result<schnorr::Signature> {
        let commitment: Option<CommitmentDBEntry> = self
            .db
            .query(
                "SELECT * FROM type::table($table)
    WHERE
        array::is_empty(array::difference(inputs, $inputs))
    AND
        array::is_empty(array::difference(outputs, $outputs))
    LIMIT 1",
            )
            .bind(("table", Self::TABLE))
            .bind(("inputs", inputs.to_vec()))
            .bind(("outputs", outputs.to_vec()))
            .await
            .map_err(|e| Error::CommitmentRepository(anyhow!("SurrealDB error: {}", e)))?
            .take(0)
            .map_err(|e| Error::CommitmentRepository(anyhow!("SurrealDB error: {}", e)))?;
        let Some(entry) = commitment else {
            return Err(Error::ResourceNotFound(format!(
                "commitment not found {inputs:?} ; {outputs:?}"
            )));
        };
        let key = entry.id.key().to_string();
        let commitment = schnorr::Signature::from_str(&key).expect("signature from recordId");
        Ok(commitment)
    }

    async fn delete(&self, commitment: schnorr::Signature) -> Result<()> {
        let rid = RecordId::from_table_key(Self::TABLE, commitment.to_string());
        let _: Option<CommitmentDBEntry> = self.db.delete(rid).await.map_err(|e| {
            Error::CommitmentRepository(anyhow!("SurrealDB error while deleting commitment: {}", e))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_common::core_tests;
    use bcr_wdc_utils::{keys::test_utils as keys_test, signatures::test_utils as signatures_test};
    use bitcoin::{
        key::rand,
        secp256k1::{self as secp, schnorr},
    };
    use persistence::{
        CommitmentRepository, KeysRepository, ProofRepository, SignaturesRepository,
    };
    use rand::Rng;

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
        let (info, keyset) = core_tests::generate_random_ecash_keyset();
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
            let (info, keyset) = core_tests::generate_random_ecash_keyset();
            let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        {
            let (info, keyset) = core_tests::generate_random_ecash_keyset();
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
            let (mut info, keyset) = core_tests::generate_random_ecash_keyset();
            info.unit = cashu::CurrencyUnit::Sat;
            info.final_expiry = Some(10);
            let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        {
            let (mut info, keyset) = core_tests::generate_random_ecash_keyset();
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
            let (mut info, keyset) = core_tests::generate_random_ecash_keyset();
            info.final_expiry = Some(10);
            let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        {
            let (mut info, keyset) = core_tests::generate_random_ecash_keyset();
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
            let (mut info, keyset) = core_tests::generate_random_ecash_keyset();
            info.final_expiry = Some(10);
            let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        {
            let (mut info, keyset) = core_tests::generate_random_ecash_keyset();
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
        let (info, keyset) = core_tests::generate_random_ecash_keyset();
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
            let (info, keyset) = core_tests::generate_random_ecash_keyset();
            let rid = RecordId::from_table_key(DBKeys::TABLE, info.id.to_string());
            let dbkeys = convert_to_keysdbentry((info.clone(), keyset), DBKeys::TABLE);
            let _r: Option<KeysDBEntry> = db.db.insert(&rid).content(dbkeys).await.unwrap();
        }
        {
            let (info, keyset) = core_tests::generate_random_ecash_keyset();
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
        let (mut info, keyset) = core_tests::generate_random_ecash_keyset();
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
        let (info, _) = core_tests::generate_random_ecash_keyset();
        let res = db.update_info(info).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    #[ignore = "SurrealDB issue #6405"]
    async fn infos_for_expiration_date() {
        let db = init_keys_mem_db().await;
        let mut keys0 = core_tests::generate_random_ecash_keyset();
        keys0.0.final_expiry = Some(30);
        keys0.1.final_expiry = keys0.0.final_expiry;
        db.store(keys0).await.unwrap();
        let mut keys1 = core_tests::generate_random_ecash_keyset();
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
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts = [cashu::Amount::from(8u64)];

        let y = keys_test::publics()[0];
        let signature = signatures_test::generate_signatures(&keyset, &amounts)[0].clone();

        db.store(y, signature).await.unwrap();
    }

    #[tokio::test]
    async fn dbsignatures_store_same_signature_twice() {
        let db = init_mem_dbsignatures().await;
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts = [cashu::Amount::from(8u64)];

        let y = keys_test::publics()[0];
        let signature = signatures_test::generate_signatures(&keyset, &amounts)[0].clone();

        db.store(y, signature.clone()).await.unwrap();
        let res = db.store(y, signature).await;
        assert!(matches!(res, Err(Error::Conflict(_))));
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
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(
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
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(
            &keyset,
            &[cashu::Amount::from(16_u64), cashu::Amount::from(8_u64)],
        );
        db.insert(&proofs).await.unwrap();

        let res = db.insert(&proofs).await;
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::InvalidInput(_)));
    }

    #[tokio::test]
    async fn test_insert_double_spent_partial() {
        let db = init_proofs_mem_db().await;
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(
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
        assert!(matches!(res.unwrap_err(), Error::InvalidInput(_)));
    }

    #[tokio::test]
    async fn test_insert_double_spent_partial_still_valid() {
        let db = init_proofs_mem_db().await;
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(
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

    async fn init_commitments_mem_db() -> DBCommitments {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DBCommitments { db: sdb }
    }

    fn random_cdk_pks(sz: usize) -> Vec<cashu::PublicKey> {
        std::iter::repeat_with(|| {
            cashu::PublicKey::from(bcr_common::core_tests::generate_random_keypair().public_key())
        })
        .take(sz)
        .collect()
    }

    fn random_signature() -> schnorr::Signature {
        let mut sl = [0; secp::constants::SCHNORR_SIGNATURE_SIZE];
        rand::thread_rng().fill(&mut sl[..]);
        schnorr::Signature::from_slice(&sl).unwrap()
    }

    fn random_wallet_key() -> cashu::PublicKey {
        let pk = secp::generate_keypair(&mut rand::thread_rng()).1;
        cashu::PublicKey::from(pk)
    }

    #[tokio::test]
    async fn store() {
        let db = init_commitments_mem_db().await;
        let inputs = random_cdk_pks(5);
        let outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = random_signature();
        db.store(inputs, outputs, tstamp, random_wallet_key(), signature)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn contains_inputs() {
        let db = init_commitments_mem_db().await;
        let inputs = random_cdk_pks(5);
        let outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = random_signature();
        db.store(
            inputs.clone(),
            outputs.clone(),
            tstamp,
            random_wallet_key(),
            signature,
        )
        .await
        .unwrap();
        let mut tester = random_cdk_pks(2);
        let result = db.contains_inputs(&tester).await;
        assert!(!result.unwrap());
        tester.push(inputs[0]);
        let result = db.contains_inputs(&tester).await;
        assert!(result.unwrap());
        let result = db.contains_inputs(&inputs).await;
        assert!(result.unwrap());
        let result = db.contains_inputs(&outputs).await;
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn contains_outputs() {
        let db = init_commitments_mem_db().await;
        let inputs = random_cdk_pks(5);
        let outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = random_signature();
        db.store(
            inputs.clone(),
            outputs.clone(),
            tstamp,
            random_wallet_key(),
            signature,
        )
        .await
        .unwrap();
        let mut tester = random_cdk_pks(2);
        let result = db.contains_outputs(&tester).await;
        assert!(!result.unwrap());
        tester.push(outputs[0]);
        let result = db.contains_outputs(&tester).await;
        assert!(result.unwrap());
        let result = db.contains_outputs(&outputs).await;
        assert!(result.unwrap());
        let result = db.contains_outputs(&inputs).await;
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn load() {
        let db = init_commitments_mem_db().await;
        let inputs = random_cdk_pks(5);
        let outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = random_signature();
        db.store(
            inputs.clone(),
            outputs.clone(),
            tstamp,
            random_wallet_key(),
            signature,
        )
        .await
        .unwrap();
        // wrong inputs
        let tester = random_cdk_pks(2);
        let result = db.load(&tester, &outputs).await;
        assert!(result.is_err());
        // wrong outputs
        let result = db.load(&inputs, &tester).await;
        assert!(result.is_err());
        let mut tester = random_cdk_pks(4);
        tester.push(inputs[0]);
        // subset of inputs
        let result = db.load(&tester, &outputs).await;
        assert!(result.is_err());
        let mut tester = random_cdk_pks(2);
        tester.push(outputs[0]);
        // subset of outputs
        let result = db.load(&inputs, &tester).await;
        assert!(result.is_err());
        // correct inputs and outputs
        let result = db.load(&inputs, &outputs).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), signature);
        let mut tester_in = inputs.clone();
        tester_in.extend(random_cdk_pks(2));
        let mut tester_out = outputs.clone();
        tester_out.extend(random_cdk_pks(2));
        // extra inputs and outputs
        let result = db.load(&tester_in, &tester_out).await;
        assert!(result.is_err());
    }
}
