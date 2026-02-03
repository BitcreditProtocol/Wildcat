// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    cashu::{self, secret::Secret, Amount},
    wire::keys as wire_keys,
};
use bitcoin::hashes::sha256::Hash as Sha256Hash;
use surrealdb::{engine::any::Any, RecordId, Result as SurrealResult, Surreal};
use uuid::Uuid;
// ----- local imports
use crate::{
    debit,
    error::{Error, Result},
    foreign, persistence,
};

// ----- end imports

// cashu::PreMint is not Deserialize
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DBEntryPremint {
    blinded: cashu::BlindedMessage,
    secret: Secret,
    r: cashu::SecretKey,
    amount: Amount,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DBEntryPremintSecret {
    request_id: Uuid,
    kid: cashu::Id,
    secrets: Vec<DBEntryPremint>,
}

impl std::convert::From<DBEntryPremint> for cashu::PreMint {
    fn from(entry: DBEntryPremint) -> Self {
        Self {
            blinded_message: entry.blinded,
            secret: entry.secret,
            r: entry.r,
            amount: entry.amount,
        }
    }
}

impl std::convert::From<cashu::PreMint> for DBEntryPremint {
    fn from(entry: cashu::PreMint) -> Self {
        Self {
            blinded: entry.blinded_message,
            secret: entry.secret,
            r: entry.r,
            amount: entry.amount,
        }
    }
}

impl std::convert::From<DBEntryPremintSecret> for cashu::PreMintSecrets {
    fn from(entry: DBEntryPremintSecret) -> Self {
        let DBEntryPremintSecret { kid, secrets, .. } = entry;
        let secrets: Vec<cashu::PreMint> = secrets.into_iter().map(|e| e.into()).collect();
        Self {
            keyset_id: kid,
            secrets,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct DebitConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub table: String,
    pub onchain_melts: String,
    pub onchain_mints: String,
}

#[derive(Debug, Clone)]
pub struct DebitRepository {
    db: Surreal<Any>,
    table: String,
    onchain_melts: String,
    onchain_mints: String,
}
impl DebitRepository {
    pub async fn new(config: DebitConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(config.connection).await?;
        db_connection.use_ns(config.namespace).await?;
        db_connection.use_db(config.database).await?;
        Ok(Self {
            db: db_connection,
            table: config.table,
            onchain_melts: config.onchain_melts,
            onchain_mints: config.onchain_mints,
        })
    }
}

#[async_trait]
impl persistence::Repository for DebitRepository {
    async fn store_quote(&self, quote: debit::MintQuote) -> Result<()> {
        let rid = RecordId::from_table_key(&self.table, quote.qid.clone());
        let _: Option<debit::MintQuote> = self
            .db
            .insert(rid)
            .content(quote)
            .await
            .map_err(Error::DB)?;
        Ok(())
    }

    async fn update_quote(&self, quote: debit::MintQuote) -> Result<()> {
        let rid = RecordId::from_table_key(&self.table, quote.qid.clone());
        let _: Option<debit::MintQuote> = self
            .db
            .update(rid)
            .content(quote)
            .await
            .map_err(Error::DB)?;
        Ok(())
    }

    async fn list_quotes(&self) -> Result<Vec<debit::MintQuote>> {
        let statement = String::from("SELECT * FROM type::table($table)");
        let entries: Vec<debit::MintQuote> = self
            .db
            .query(statement)
            .bind(("table", self.table.clone()))
            .await
            .map_err(Error::DB)?
            .take(0)
            .map_err(Error::DB)?;
        Ok(entries)
    }

    async fn store_onchain_melt(
        &self,
        quote_id: uuid::Uuid,
        data: debit::OnchainMeltQuote,
    ) -> Result<()> {
        let rid = RecordId::from_table_key(&self.onchain_melts, quote_id);
        let _: Option<debit::OnchainMeltQuote> =
            self.db.insert(rid).content(data).await.map_err(Error::DB)?;
        Ok(())
    }

    async fn load_onchain_melt(&self, quote_id: uuid::Uuid) -> Result<debit::OnchainMeltQuote> {
        let rid = RecordId::from_table_key(&self.onchain_melts, quote_id);
        let result: Option<debit::OnchainMeltQuote> =
            self.db.select(rid).await.map_err(Error::DB)?;
        result.ok_or_else(|| Error::RequestIDNotFound(quote_id))
    }

    async fn store_onchain_mint(
        &self,
        quote_id: uuid::Uuid,
        data: debit::ClowderMintQuoteOnchain,
    ) -> Result<()> {
        let rid = RecordId::from_table_key(&self.onchain_mints, quote_id);
        let _: Option<debit::ClowderMintQuoteOnchain> =
            self.db.insert(rid).content(data).await.map_err(Error::DB)?;
        Ok(())
    }

    async fn load_onchain_mint(
        &self,
        quote_id: uuid::Uuid,
    ) -> Result<debit::ClowderMintQuoteOnchain> {
        let rid = RecordId::from_table_key(&self.onchain_mints, quote_id);
        let result: Option<debit::ClowderMintQuoteOnchain> =
            self.db.select(rid).await.map_err(Error::DB)?;
        result.ok_or_else(|| Error::RequestIDNotFound(quote_id))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ForeignProofEntry {
    id: RecordId,
    proof: cashu::Proof,
    mint_url: cashu::MintUrl,
    mint_pk: secp256k1::PublicKey,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ForeignOnlineHtlcProofEntry {
    id: RecordId,
    proof: cashu::Proof,
    mint_url: cashu::MintUrl,
    mint_pk: secp256k1::PublicKey,
    hash: Sha256Hash,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ForeignOnlineConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub foreigns_table: String,
    pub htlcs_table: String,
}

#[derive(Debug, Clone)]
pub struct ForeignOnlineRepository {
    db: Surreal<Any>,
    foreigns_table: String,
    htlcs_table: String,
}

impl ForeignOnlineRepository {
    pub async fn new(config: ForeignOnlineConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(config.connection).await?;
        db_connection.use_ns(config.namespace).await?;
        db_connection.use_db(config.database).await?;
        Ok(Self {
            db: db_connection,
            foreigns_table: config.foreigns_table,
            htlcs_table: config.htlcs_table,
        })
    }
}

#[async_trait]
impl foreign::OnlineRepository for ForeignOnlineRepository {
    async fn store(
        &self,
        (mint_pk, mint_url): (secp256k1::PublicKey, cashu::MintUrl),
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let mut entries: Vec<ForeignProofEntry> = Vec::with_capacity(proofs.len());
        for proof in proofs.into_iter() {
            let rid = RecordId::from_table_key(&self.foreigns_table, proof.y()?.to_string());
            entries.push(ForeignProofEntry {
                id: rid,
                proof,
                mint_pk,
                mint_url: mint_url.clone(),
            });
        }
        let _: Vec<ForeignProofEntry> = self
            .db
            .insert(&self.foreigns_table)
            .content(entries)
            .await
            .map_err(Error::DB)?;
        Ok(())
    }

    async fn list(&self) -> Result<Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>> {
        let statement = String::from("SELECT * FROM type::table($table)");
        let entries: Vec<ForeignProofEntry> = self
            .db
            .query(statement)
            .bind(("table", self.foreigns_table.clone()))
            .await
            .map_err(Error::DB)?
            .take(0)
            .map_err(Error::DB)?;
        let mut ret_val = Vec::with_capacity(entries.len());
        for entry in entries {
            let ForeignProofEntry {
                mint_url,
                mint_pk,
                proof,
                ..
            } = entry;
            ret_val.push(((mint_pk, mint_url), proof));
        }
        Ok(ret_val)
    }

    async fn store_htlc(
        &self,
        (mint_pk, mint_url): (secp256k1::PublicKey, cashu::MintUrl),
        hash: Sha256Hash,
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let mut entries: Vec<ForeignOnlineHtlcProofEntry> = Vec::with_capacity(proofs.len());
        for proof in proofs {
            let id = RecordId::from_table_key(&self.htlcs_table, proof.y()?.to_string());
            let entry = ForeignOnlineHtlcProofEntry {
                hash,
                id,
                proof,
                mint_pk,
                mint_url: mint_url.clone(),
            };
            entries.push(entry);
        }
        let _: Vec<ForeignOnlineHtlcProofEntry> = self
            .db
            .insert(&self.htlcs_table)
            .content(entries)
            .await
            .map_err(Error::DB)?;
        Ok(())
    }

    async fn search_htlc(
        &self,
        hash: &Sha256Hash,
    ) -> Result<Vec<((secp256k1::PublicKey, cashu::MintUrl), cashu::Proof)>> {
        let htlcs: Vec<ForeignOnlineHtlcProofEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE hash = $hash")
            .bind(("table", self.htlcs_table.clone()))
            .bind(("hash", *hash))
            .await
            .map_err(Error::DB)?
            .take(0)
            .map_err(Error::DB)?;
        let ret_val = htlcs
            .into_iter()
            .map(
                |ForeignOnlineHtlcProofEntry {
                     proof,
                     mint_url,
                     mint_pk,
                     ..
                 }| ((mint_pk, mint_url), proof),
            )
            .collect();
        Ok(ret_val)
    }

    async fn remove_htlcs(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        for y in ys {
            let rid = RecordId::from_table_key(&self.htlcs_table, y.to_string());
            let _: Option<ForeignOnlineHtlcProofEntry> =
                self.db.delete(rid).await.map_err(Error::DB)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ForeignOfflineConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
    pub fps_table: String,
    pub proofs_table: String,
}

#[derive(Debug, Clone)]
pub struct ForeignOfflineRepository {
    db: Surreal<Any>,
    fps_table: String,
    proofs_table: String,
}

impl ForeignOfflineRepository {
    pub async fn new(config: ForeignOfflineConnectionConfig) -> SurrealResult<Self> {
        let db_connection = Surreal::<Any>::init();
        db_connection.connect(config.connection).await?;
        db_connection.use_ns(config.namespace).await?;
        db_connection.use_db(config.database).await?;
        Ok(Self {
            db: db_connection,
            fps_table: config.fps_table,
            proofs_table: config.proofs_table,
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ForeignFingerprintEntry {
    id: RecordId,
    amount: u64,
    keyset_id: cashu::Id,
    y: cashu::PublicKey,
    c: cashu::PublicKey,
    witness: Option<cashu::Witness>,
    dleq: Option<cashu::ProofDleq>,
    mint_pk: secp256k1::PublicKey,
    mint_url: cashu::MintUrl,
}

#[async_trait]
impl foreign::OfflineRepository for ForeignOfflineRepository {
    async fn store_fps(
        &self,
        (mint_pk, mint_url): (secp256k1::PublicKey, cashu::MintUrl),
        fps: Vec<wire_keys::ProofFingerprint>,
        hash: Vec<Sha256Hash>,
    ) -> Result<()> {
        for (hash, fp) in hash.into_iter().zip(fps.into_iter()) {
            let rid = RecordId::from_table_key(&self.fps_table, hash.to_string());
            let entry = ForeignFingerprintEntry {
                id: rid.clone(),
                amount: fp.amount,
                keyset_id: fp.keyset_id,
                y: fp.y,
                c: fp.c,
                witness: fp.witness,
                dleq: fp.dleq,
                mint_pk,
                mint_url: mint_url.clone(),
            };
            let _: Option<ForeignFingerprintEntry> = self
                .db
                .insert(rid)
                .content(entry)
                .await
                .map_err(Error::DB)?;
        }
        Ok(())
    }

    async fn search_fp(
        &self,
        hash: &Sha256Hash,
    ) -> Result<
        Option<(
            (secp256k1::PublicKey, cashu::MintUrl),
            wire_keys::ProofFingerprint,
        )>,
    > {
        let rid = RecordId::from_table_key(&self.fps_table, hash.to_string());
        let entry: Option<ForeignFingerprintEntry> = self.db.select(rid).await?;
        let Some(entry) = entry else {
            return Ok(None);
        };
        let fp = wire_keys::ProofFingerprint {
            amount: entry.amount,
            keyset_id: entry.keyset_id,
            y: entry.y,
            c: entry.c,
            witness: entry.witness,
            dleq: entry.dleq,
        };
        Ok(Some(((entry.mint_pk, entry.mint_url), fp)))
    }

    async fn remove_fps(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        let _: Vec<ForeignFingerprintEntry> = self
            .db
            .query("DELETE FROM type::table($table) WHERE array::any($ys, y)")
            .bind(("table", self.fps_table.clone()))
            .bind(("ys", ys.to_vec()))
            .await
            .map_err(Error::DB)?
            .take(0)
            .map_err(Error::DB)?;
        Ok(())
    }
    async fn store_proofs(
        &self,
        (mint_pk, mint_url): (secp256k1::PublicKey, cashu::MintUrl),
        proofs: Vec<cashu::Proof>,
    ) -> Result<()> {
        let mut entries: Vec<ForeignProofEntry> = Vec::with_capacity(proofs.len());
        for proof in proofs.into_iter() {
            let rid = RecordId::from_table_key(&self.proofs_table, proof.y()?.to_string());
            let entry = ForeignProofEntry {
                id: rid,
                proof,
                mint_pk,
                mint_url: mint_url.clone(),
            };
            entries.push(entry);
        }
        let _: Vec<ForeignProofEntry> = self.db.insert(&self.proofs_table).content(entries).await?;
        Ok(())
    }

    async fn load_proofs(
        &self,
        (mint_pk, mint_url): &(secp256k1::PublicKey, cashu::MintUrl),
    ) -> Result<Vec<cashu::Proof>> {
        let entries: Vec<ForeignProofEntry> = self
            .db
            .query("SELECT * FROM type::table($table) WHERE mint_url = $mint_url AND mint_pk = $mint_pk")
            .bind(("table", self.proofs_table.clone()))
            .bind(("mint_url", mint_url.clone()))
            .bind(("mint_pk", *mint_pk))
            .await
            .map_err(Error::DB)?
            .take(0)
            .map_err(Error::DB)?;
        let mut ret_val = Vec::with_capacity(entries.len());
        for entry in entries {
            ret_val.push(entry.proof);
        }
        Ok(ret_val)
    }

    async fn remove_proofs(&self, ys: &[cashu::PublicKey]) -> Result<()> {
        let _: Vec<ForeignProofEntry> = self
            .db
            .query("DELETE FROM type::table($table) WHERE array::any($ys, proof.y)")
            .bind(("table", self.proofs_table.clone()))
            .bind(("ys", ys.to_vec()))
            .await
            .map_err(Error::DB)?
            .take(0)
            .map_err(Error::DB)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foreign::OfflineRepository;
    use crate::persistence::Repository;
    use bcr_common::core_tests;
    use bcr_common::core_tests::generate_random_keypair;
    use bitcoin::hashes::Hash;
    use std::str::FromStr;

    async fn init_deb_mem_db() -> DebitRepository {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        DebitRepository {
            db: sdb,
            table: String::from("test"),
            onchain_melts: String::from("onchain_melts"),
            onchain_mints: String::from("onchain_mints"),
        }
    }

    #[tokio::test]
    async fn test_mint_quote() {
        let db = init_deb_mem_db().await;

        let quote = debit::MintQuote {
            qid: Uuid::new_v4().to_string(),
            ebill_id: core_tests::random_bill_id(),
            clowder_qid: Uuid::new_v4(),
            mint_complete: false,
        };
        db.store_quote(quote.clone()).await.unwrap();

        let list = db.list_quotes().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].qid, quote.qid);

        db.update_quote(quote).await.unwrap();
    }

    async fn init_foreignoffline_mem_db() -> ForeignOfflineRepository {
        let sdb = Surreal::<Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        ForeignOfflineRepository {
            db: sdb,
            fps_table: String::from("fps_table"),
            proofs_table: String::from("proofs_table"),
        }
    }

    #[tokio::test]
    async fn offline_search_fps() {
        let db = init_foreignoffline_mem_db().await;

        let alpha_pk = generate_random_keypair().public_key();
        let alpha = (
            alpha_pk,
            cashu::MintUrl::from_str("http://example.com").unwrap(),
        );
        let y = cashu::PublicKey::from(generate_random_keypair().public_key());
        let c = cashu::PublicKey::from(generate_random_keypair().public_key());
        let fps = vec![
            wire_keys::ProofFingerprint {
                amount: 10,
                keyset_id: cashu::Id::from_bytes(&[1; 33]).unwrap(),
                y,
                c,
                witness: None,
                dleq: None,
            },
            wire_keys::ProofFingerprint {
                amount: 10,
                keyset_id: cashu::Id::from_bytes(&[1; 33]).unwrap(),
                y: cashu::PublicKey::from(generate_random_keypair().public_key()),
                c: cashu::PublicKey::from(generate_random_keypair().public_key()),
                witness: None,
                dleq: None,
            },
        ];
        let hash = vec![
            Sha256Hash::from_slice(&[0u8; 32]).unwrap(),
            Sha256Hash::from_slice(&[1u8; 32]).unwrap(),
        ];
        db.store_fps(alpha.clone(), fps, hash.clone())
            .await
            .unwrap();
        let result = db.search_fp(&hash[0]).await.unwrap();
        assert!(result.is_some());
        let (mint, fp) = result.unwrap();
        assert_eq!(mint.0, alpha.0);
        assert_eq!(fp.y, y);
    }
}
