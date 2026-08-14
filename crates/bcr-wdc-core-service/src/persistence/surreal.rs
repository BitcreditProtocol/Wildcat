// ----- standard library imports
use std::{
    collections::{BTreeMap, HashMap, HashSet},
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
    client::admin::core::{BRError, RNFError},
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

fn cpk_to_record_id(main_table: &str, pk: cashu::PublicKey) -> RecordId {
    RecordId::from_table_key(main_table, pk.to_string())
}

const KEYS_TABLE: &str = "keys";
const SIGNATURES_TABLE: &str = "signatures";
const PROOFS_TABLE: &str = "proofs";
const COMMITMENTS_TABLE: &str = "commitments";
const RESERVED_YS_TABLE: &str = "reserved_ys";

#[derive(Debug, Clone)]
pub struct Repository {
    db: Surreal<Any>,
}

impl Repository {
    pub async fn new(repository: surreal::DBConnConfig) -> SurrealResult<Self> {
        Ok(Self {
            db: connect_surreal(repository).await?,
        })
    }

    #[cfg(test)]
    pub(in crate::persistence) fn from_db(db: Surreal<Any>) -> Self {
        Self { db }
    }
}

async fn connect_surreal(cfg: surreal::DBConnConfig) -> SurrealResult<Surreal<Any>> {
    let db_connection = Surreal::<Any>::init();
    db_connection.connect(cfg.connection).await?;
    db_connection.use_ns(cfg.namespace).await?;
    db_connection.use_db(cfg.database).await?;
    Ok(db_connection)
}

#[async_trait]
impl persistence::Repository for Repository {
    async fn keys_store(&self, keys: KeysetEntry) -> Result<()> {
        Repository::keys_store(self, keys).await
    }

    async fn keys_info(&self, id: cashu::Id) -> Result<Option<MintKeySetInfo>> {
        Repository::keys_info(self, id).await
    }

    async fn keys_load(&self, id: cashu::Id) -> Result<Option<cashu::MintKeySet>> {
        Repository::keys_load(self, id).await
    }

    async fn keys_list_info(
        &self,
        currency: Option<cashu::CurrencyUnit>,
        min_expiration_tstamp: Option<u64>,
        max_expiration_tstamp: Option<u64>,
    ) -> Result<Vec<MintKeySetInfo>> {
        Repository::keys_list_info(self, currency, min_expiration_tstamp, max_expiration_tstamp)
            .await
    }

    async fn keys_infos_for_expiration_date(&self, expire: u64) -> Result<Vec<MintKeySetInfo>> {
        Repository::keys_infos_for_expiration_date(self, expire).await
    }

    async fn signature_store(
        &self,
        y: cashu::PublicKey,
        signature: cashu::BlindSignature,
    ) -> Result<()> {
        Repository::signature_store(self, y, signature).await
    }

    async fn signature_load(
        &self,
        blind: &cashu::BlindedMessage,
    ) -> Result<Option<cashu::BlindSignature>> {
        Repository::signature_load(self, blind).await
    }

    async fn proofs_insert(&self, tokens: Vec<cashu::Proof>) -> Result<()> {
        Repository::proofs_insert(self, tokens).await
    }

    async fn proofs_remove(&self, tokens: &[cashu::PublicKey]) -> Result<()> {
        Repository::proofs_remove(self, tokens).await
    }

    async fn proofs_contains(&self, y: cashu::PublicKey) -> Result<Option<cashu::ProofState>> {
        Repository::proofs_contains(self, y).await
    }

    async fn commitment_store(
        &self,
        inputs: Vec<cashu::PublicKey>,
        outputs: Vec<cashu::PublicKey>,
        expiration: TStamp,
        wallet_key: cashu::PublicKey,
        commitment: schnorr::Signature,
        fp_digest: [u8; 32],
        signed: persistence::SignatureOwner,
    ) -> Result<()> {
        Repository::commitment_store(
            self, inputs, outputs, expiration, wallet_key, commitment, fp_digest, signed,
        )
        .await
    }

    async fn commitment_load(
        &self,
        signature: &schnorr::Signature,
    ) -> Result<persistence::StoredCommitment> {
        Repository::commitment_load(self, signature).await
    }

    async fn commitment_contains_inputs(&self, inputs: &[cashu::PublicKey]) -> Result<bool> {
        Repository::commitment_contains_inputs(self, inputs).await
    }

    async fn commitment_contains_outputs(&self, outputs: &[cashu::PublicKey]) -> Result<bool> {
        Repository::commitment_contains_outputs(self, outputs).await
    }

    async fn commitment_delete(&self, commitment: schnorr::Signature) -> Result<()> {
        Repository::commitment_delete(self, commitment).await
    }

    async fn commitment_clean_expired(&self, now: TStamp) -> Result<()> {
        Repository::commitment_clean_expired(self, now).await
    }

    async fn ys_store(&self, inputs: Vec<cashu::PublicKey>, deadline: TStamp) -> Result<()> {
        Repository::ys_store(self, inputs, deadline).await
    }

    async fn ys_contains(&self, inputs: &[cashu::PublicKey]) -> Result<Vec<bool>> {
        Repository::ys_contains(self, inputs).await
    }

    async fn ys_clean_expired(&self, now: TStamp) -> Result<()> {
        Repository::ys_clean_expired(self, now).await
    }
}

//////////////////////////////////////////////////////////////////////// Keys DB
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct KeysInfoDBEntry {
    kid: cashu::Id,
    unit: cashu::CurrencyUnit,
    active: bool,
    valid_from: u64,
    derivation_path: DerivationPath,
    derivation_path_index: Option<u32>,
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
            input_fee_ppk: info.input_fee_ppk,
            final_expiry: info.final_expiry,
            amounts: info.denominations,
            issuer_version: None,
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
            input_fee_ppk: info.input_fee_ppk,
            final_expiry: info.final_expiry,
        };
        (info, keyset)
    }
}

impl Repository {
    pub async fn dump_keys(&self) -> Result<Vec<KeysetEntry>> {
        let entries: Vec<KeysDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table)")
            .bind(("table", KEYS_TABLE))
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        Ok(entries.into_iter().map(KeysetEntry::from).collect())
    }
    async fn keys_store(&self, entry: KeysetEntry) -> Result<()> {
        let rid = RecordId::from_table_key(KEYS_TABLE, entry.0.id.to_string());
        let dbentry = convert_to_keysdbentry(entry, KEYS_TABLE);
        let _resp: Option<KeysDBEntry> = self
            .db
            .insert(rid)
            .content(dbentry)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        Ok(())
    }

    async fn keys_info(&self, kid: cashu::Id) -> Result<Option<MintKeySetInfo>> {
        let rid = RecordId::from_table_key(KEYS_TABLE, kid.to_string());
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

    async fn keys_load(&self, kid: cashu::Id) -> Result<Option<cashu::MintKeySet>> {
        let rid = RecordId::from_table_key(KEYS_TABLE, kid.to_string());
        let entry: Option<KeysDBEntry> = self
            .db
            .select(rid)
            .await
            .map_err(|e| Error::KeysRepository(anyhow!(e)))?;
        let keyset = entry.map(KeysetEntry::from).map(|(_, keyset)| keyset);
        Ok(keyset)
    }

    async fn keys_list_info(
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
            .bind(("table", KEYS_TABLE))
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

    async fn keys_infos_for_expiration_date(&self, expire: u64) -> Result<Vec<MintKeySetInfo>> {
        let infos: Vec<KeysInfoDBEntry> = self
            .db
            // WARNING: https://github.com/surrealdb/surrealdb/issues/6405
            // .query("SELECT info FROM type::table($table) WHERE info.final_expiry > $tstamp ORDER BY info.final_expiry ASC")
            .query("SELECT VALUE info FROM type::table($table) WHERE info.final_expiry >= $expire")
            .bind(("table", KEYS_TABLE))
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
    pub id: RecordId,
    pub amount: cashu::Amount,
    pub keyset_id: cashu::Id,
    pub c: cashu::PublicKey,
    pub dleq: Option<cashu::BlindSignatureDleq>,
}
fn convert_to_entry(rid: RecordId, sig: cashu::BlindSignature) -> SignatureDBEntry {
    SignatureDBEntry {
        id: rid,
        amount: sig.amount,
        keyset_id: sig.keyset_id,
        c: sig.c,
        dleq: sig.dleq,
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

impl Repository {
    pub async fn dump_signatures(&self) -> Result<Vec<(cashu::PublicKey, cashu::BlindSignature)>> {
        let entries: Vec<SignatureDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table)")
            .bind(("table", SIGNATURES_TABLE))
            .await
            .map_err(|e| Error::SignaturesRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::SignaturesRepository(anyhow!(e)))?;
        let result = entries
            .into_iter()
            .map(|entry| {
                let y = cashu::PublicKey::from_str(&entry.id.key().to_string())
                    .expect("RecordKey is y");
                (y, cashu::BlindSignature::from(entry))
            })
            .collect::<Vec<(cashu::PublicKey, cashu::BlindSignature)>>();
        Ok(result)
    }
    async fn signature_store(
        &self,
        y: cashu::PublicKey,
        signature: cashu::BlindSignature,
    ) -> Result<()> {
        let rid = cpk_to_record_id(SIGNATURES_TABLE, y);
        let entry = convert_to_entry(rid.clone(), signature);
        let r: SurrealResult<Option<SignatureDBEntry>> = self.db.insert(rid).content(entry).await;
        match r {
            Err(SurrealError::Db(SurrealDBError::RecordExists { .. })) => {
                Err(Error::Conflict(format!("signature already exists: {y}")))
            }
            Err(e) => Err(Error::SignaturesRepository(anyhow!(e))),
            Ok(..) => Ok(()),
        }
    }
    async fn signature_load(
        &self,
        blind: &cashu::BlindedMessage,
    ) -> Result<Option<cashu::BlindSignature>> {
        let rid = cpk_to_record_id(SIGNATURES_TABLE, blind.blinded_secret);
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
pub struct ProofDBEntry {
    pub id: RecordId,
    pub kid: cashu::Id,
    pub secret: cashu::secret::Secret,
    pub c: cashu::PublicKey,
    pub witness: Option<cashu::Witness>,
}
fn convert_to_db(proof: cashu::Proof, table: &str) -> Result<ProofDBEntry> {
    let rid = cpk_to_record_id(table, proof.y()?);
    let dbentry = ProofDBEntry {
        id: rid,
        kid: proof.keyset_id,
        secret: proof.secret,
        c: proof.c,
        witness: proof.witness,
    };
    Ok(dbentry)
}

impl Repository {
    pub async fn dump_proofs(&self) -> Result<Vec<ProofDBEntry>> {
        let entries: Vec<ProofDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table)")
            .bind(("table", PROOFS_TABLE))
            .await
            .map_err(|e| Error::ProofRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::ProofRepository(anyhow!(e)))?;
        Ok(entries)
    }
    async fn proofs_insert(&self, tokens: Vec<cashu::Proof>) -> Result<()> {
        let mut entries: Vec<ProofDBEntry> = Vec::with_capacity(tokens.len());
        let mut ys = HashSet::with_capacity(tokens.len());
        for tk in tokens {
            let y = tk.y()?;
            if !ys.insert(y) {
                return Err(Error::InvalidInput(BRError::Generic(String::from(
                    "proofs already spent",
                ))));
            }
            let db_entry = convert_to_db(tk, PROOFS_TABLE)?;
            entries.push(db_entry);
        }
        let _: Vec<ProofDBEntry> =
            self.db
                .insert(())
                .content(entries)
                .await
                .map_err(|e| match e {
                    surrealdb::Error::Db(surrealdb::error::Db::RecordExists { .. }) => {
                        Error::InvalidInput(BRError::Generic(String::from("proofs already spent")))
                    }
                    _ => Error::ProofRepository(anyhow!(e)),
                })?;
        Ok(())
    }

    async fn proofs_remove(&self, tokens: &[cashu::PublicKey]) -> Result<()> {
        for tk in tokens {
            let rid = cpk_to_record_id(PROOFS_TABLE, *tk);
            let _p: Option<cashu::Proof> = self
                .db
                .delete(rid)
                .await
                .map_err(|e| Error::ProofRepository(anyhow!(e)))?;
        }
        Ok(())
    }

    async fn proofs_contains(&self, y: cashu::PublicKey) -> Result<Option<cashu::ProofState>> {
        let rid = cpk_to_record_id(PROOFS_TABLE, y);
        let res: Option<ProofDBEntry> = self
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

////////////////////////////////////////////////////////////////////// Commitments DB
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SignatureOwner {
    Unsigned,
    Alpha,
    Beta,
}
impl std::convert::From<persistence::SignatureOwner> for SignatureOwner {
    fn from(owner: persistence::SignatureOwner) -> Self {
        match owner {
            persistence::SignatureOwner::Unsigned => SignatureOwner::Unsigned,
            persistence::SignatureOwner::Alpha => SignatureOwner::Alpha,
            persistence::SignatureOwner::Beta => SignatureOwner::Beta,
        }
    }
}
impl std::convert::From<SignatureOwner> for persistence::SignatureOwner {
    fn from(owner: SignatureOwner) -> Self {
        match owner {
            SignatureOwner::Unsigned => persistence::SignatureOwner::Unsigned,
            SignatureOwner::Alpha => persistence::SignatureOwner::Alpha,
            SignatureOwner::Beta => persistence::SignatureOwner::Beta,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CommitmentDBEntry {
    id: RecordId,
    inputs: Vec<cashu::PublicKey>,
    outputs: Vec<cashu::PublicKey>,
    expiration: TStamp,
    wallet_key: cashu::PublicKey,
    fp_digest: [u8; 32],
    signed: SignatureOwner,
}

impl Repository {
    async fn commitment_clean_expired(&self, now: TStamp) -> Result<()> {
        self.db
            .query("DELETE FROM type::table($table) WHERE expiration < $now")
            .bind(("table", COMMITMENTS_TABLE))
            .bind(("now", now))
            .await
            .map_err(|e| Error::CommitmentRepository(anyhow!(e)))?;
        Ok(())
    }

    async fn commitment_contains_inputs(&self, ys: &[cashu::PublicKey]) -> Result<bool> {
        let commitment: Option<CommitmentDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE array::is_empty(array::intersect(inputs, $ys)) = false LIMIT 1")
            .bind(("table", COMMITMENTS_TABLE))
            .bind(("ys", ys.to_vec()))
            .await
            .map_err(|e| Error::CommitmentRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::CommitmentRepository(anyhow!(e)))?;
        Ok(commitment.is_some())
    }

    async fn commitment_contains_outputs(&self, secrets: &[cashu::PublicKey]) -> Result<bool> {
        let commitment: Option<CommitmentDBEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE array::is_empty(array::intersect(outputs, $secrets)) = false LIMIT 1")
            .bind(("table", COMMITMENTS_TABLE))
            .bind(("secrets", secrets.to_vec()))
            .await
            .map_err(|e| Error::CommitmentRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::CommitmentRepository(anyhow!(e)))?;
        Ok(commitment.is_some())
    }

    async fn commitment_store(
        &self,
        inputs: Vec<cashu::PublicKey>,
        outputs: Vec<cashu::PublicKey>,
        expiration: TStamp,
        wallet_key: cashu::PublicKey,
        signature: schnorr::Signature,
        fp_digest: [u8; 32],
        signed: persistence::SignatureOwner,
    ) -> Result<()> {
        let rid = RecordId::from_table_key(COMMITMENTS_TABLE, signature.to_string());
        let existing: Option<CommitmentDBEntry> = self
            .db
            .select(rid.clone())
            .await
            .map_err(|e| Error::CommitmentRepository(anyhow!(e)))?;
        if existing.is_some() {
            return Err(Error::Conflict(format!(
                "commitment already exists: {signature}"
            )));
        }
        let newentry = CommitmentDBEntry {
            id: rid,
            inputs,
            outputs,
            expiration,
            wallet_key,
            fp_digest,
            signed: signed.into(),
        };
        let mut query = self
            .db
            .query(
                "
    BEGIN;
        LET $no_inputs = array::is_empty(
            SELECT inputs
            FROM type::table($table)
            WHERE !array::is_empty(array::intersect(inputs, $inputs))
        );
        LET $no_outputs = array::is_empty(
            SELECT outputs
            FROM type::table($table)
            WHERE !array::is_empty(array::intersect(outputs, $outputs))
        );
        IF $no_inputs && $no_outputs {
            INSERT $content
        };
        SELECT * FROM $newrid;
    COMMIT
            ",
            )
            .bind(("table", COMMITMENTS_TABLE))
            .bind(("inputs", newentry.inputs.clone()))
            .bind(("outputs", newentry.outputs.clone()))
            .bind(("newrid", newentry.id.clone()))
            .bind(("content", newentry))
            .await
            .map_err(|e| Error::CommitmentRepository(anyhow!(e)))?;
        let inserted: Option<CommitmentDBEntry> = query
            .take(query.num_statements() - 1)
            .map_err(|e| Error::CommitmentRepository(anyhow!(e)))?;
        if inserted.is_none() {
            Err(Error::Conflict(String::from(
                "commitment with same inputs or outputs already exists",
            )))
        } else {
            Ok(())
        }
    }

    async fn commitment_load(
        &self,
        signature: &schnorr::Signature,
    ) -> Result<persistence::StoredCommitment> {
        let rid = RecordId::from_table_key(COMMITMENTS_TABLE, signature.to_string());
        let commitment_entry: CommitmentDBEntry = self
            .db
            .select(rid.clone())
            .await
            .map_err(|e| Error::CommitmentRepository(anyhow!(e)))?
            .ok_or(Error::ResourceNotFound(RNFError::Generic(rid.to_string())))?;
        Ok(persistence::StoredCommitment {
            inputs: commitment_entry.inputs,
            outputs: commitment_entry.outputs,
            expiration: commitment_entry.expiration,
            fp_digest: commitment_entry.fp_digest,
            signed: commitment_entry.signed.into(),
        })
    }

    async fn commitment_delete(&self, commitment: schnorr::Signature) -> Result<()> {
        let rid = RecordId::from_table_key(COMMITMENTS_TABLE, commitment.to_string());
        let _: Option<CommitmentDBEntry> = self
            .db
            .delete(rid)
            .await
            .map_err(|e| Error::CommitmentRepository(anyhow!(e)))?;
        Ok(())
    }
}

////////////////////////////////////////////////////////////////////////// Reserved Ys DB
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReservedYsDBEntry {
    id: RecordId,
    deadline: TStamp,
}

impl Repository {
    async fn ys_store(&self, inputs: Vec<cashu::PublicKey>, deadline: TStamp) -> Result<()> {
        let mut entries = Vec::with_capacity(inputs.len());
        for y in inputs {
            let rid = cpk_to_record_id(RESERVED_YS_TABLE, y);
            let entry = ReservedYsDBEntry { id: rid, deadline };
            entries.push(entry);
        }
        let _: Vec<ReservedYsDBEntry> = self
            .db
            .insert(RESERVED_YS_TABLE)
            .content(entries)
            .await
            .map_err(|e| match e {
                surrealdb::Error::Db(surrealdb::error::Db::RecordExists { .. }) => {
                    Error::Conflict(String::from("ys already reserved"))
                }
                _ => Error::ReservedYsRepository(anyhow!(e)),
            })?;
        Ok(())
    }

    async fn ys_contains(&self, inputs: &[cashu::PublicKey]) -> Result<Vec<bool>> {
        let rids: Vec<RecordId> = inputs
            .iter()
            .map(|y| cpk_to_record_id(RESERVED_YS_TABLE, *y))
            .collect();
        let reserved: Vec<RecordId> = self
            .db
            .query("SELECT VALUE id FROM type::table($table) WHERE id IN $rids")
            .bind(("table", RESERVED_YS_TABLE))
            .bind(("rids", rids.clone()))
            .await
            .map_err(|e| Error::ReservedYsRepository(anyhow!(e)))?
            .take(0)
            .map_err(|e| Error::ReservedYsRepository(anyhow!(e)))?;
        let reserved_set: HashSet<RecordId> = reserved.into_iter().collect();
        let mut result = Vec::with_capacity(inputs.len());
        for rid in rids {
            result.push(reserved_set.contains(&rid));
        }
        Ok(result)
    }

    async fn ys_clean_expired(&self, now: TStamp) -> Result<()> {
        self.db
            .query("DELETE FROM type::table($table) WHERE deadline < $now")
            .bind(("table", RESERVED_YS_TABLE))
            .bind(("now", now))
            .await
            .map_err(|e| Error::ReservedYsRepository(anyhow!(e)))?;
        Ok(())
    }
}
