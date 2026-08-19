// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, ecash};
use bcr_wdc_utils::keys as keys_utils;
use bitcoin::secp256k1::schnorr;
// ----- local imports
use crate::{error::Result, TStamp};
// ----- local modules
pub mod inmemory;
pub mod sqlx;
pub mod surreal;

// ----- end imports

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository: Send + Sync {
    async fn keys_store(&self, keys: keys_utils::MintKeysEntry) -> Result<()>;
    async fn keys_info(&self, id: cashu::Id) -> Result<Option<ecash::MintKeySetInfo>>;
    async fn keys_load(&self, id: cashu::Id) -> Result<Option<ecash::MintKeySet>>;
    async fn keys_list_info(
        &self,
        currency: Option<cashu::CurrencyUnit>,
        min_expiration_tstamp: Option<u64>,
        max_expiration_tstamp: Option<u64>,
    ) -> Result<Vec<ecash::MintKeySetInfo>>;
    async fn keys_infos_for_expiration_date(
        &self,
        expire: u64,
    ) -> Result<Vec<ecash::MintKeySetInfo>>;
    async fn signature_store(
        &self,
        y: cashu::PublicKey,
        signature: cashu::BlindSignature,
    ) -> Result<()>;
    async fn signature_load(
        &self,
        blind: &cashu::BlindedMessage,
    ) -> Result<Option<cashu::BlindSignature>>;
    /// Atomically completes the local state transition for an acknowledged swap.
    ///
    /// On failure, no proofs or signatures are inserted and the commitment is retained.
    async fn swap_finalize(
        &self,
        proofs: Vec<cashu::Proof>,
        signatures: Vec<StoredSignature>,
        commitment: schnorr::Signature,
    ) -> Result<()>;
    /// WARNING: this method should do strict insert.
    /// i.e. it should fail if any of the proofs is already present in the DB
    /// in case of failure, the DB should be in the same state as before the call
    async fn proofs_insert(&self, tokens: Vec<cashu::Proof>) -> Result<()>;
    async fn proofs_remove(&self, tokens: &[cashu::PublicKey]) -> Result<()>;
    async fn proofs_contains(&self, y: cashu::PublicKey) -> Result<Option<cashu::ProofState>>;
    async fn commitment_store(
        &self,
        inputs: Vec<cashu::PublicKey>,
        outputs: Vec<cashu::PublicKey>,
        expiration: TStamp,
        wallet_key: cashu::PublicKey,
        commitment: schnorr::Signature,
        fp_digest: [u8; 32],
        signed: SignatureOwner,
    ) -> Result<()>;
    async fn commitment_load(&self, signature: &schnorr::Signature) -> Result<StoredCommitment>;
    async fn commitment_contains_inputs(&self, inputs: &[cashu::PublicKey]) -> Result<bool>;
    async fn commitment_contains_outputs(&self, outputs: &[cashu::PublicKey]) -> Result<bool>;
    async fn commitment_delete(&self, commitment: schnorr::Signature) -> Result<()>;
    async fn commitment_clean_expired(&self, now: TStamp) -> Result<()>;
    async fn ys_store(&self, inputs: Vec<cashu::PublicKey>, deadline: TStamp) -> Result<()>;
    async fn ys_contains(&self, inputs: &[cashu::PublicKey]) -> Result<Vec<bool>>;
    async fn ys_clean_expired(&self, now: TStamp) -> Result<()>;
    // no need to delete as inputs can only end up being burnt, and they will appear as spent in
    // ProofRepository, or they will be cleaned up after the deadline by clean_expired
}

#[derive(Clone, Debug)]
pub struct StoredSignature {
    pub y: cashu::PublicKey,
    pub signature: cashu::BlindSignature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SignatureOwner {
    Unsigned,
    Alpha,
    Beta,
}

pub struct StoredCommitment {
    pub inputs: Vec<cashu::PublicKey>,
    pub outputs: Vec<cashu::PublicKey>,
    pub expiration: TStamp,
    pub fp_digest: [u8; 32],
    pub signed: SignatureOwner,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use bcr_common::{cashu::ProofsMethods, cdk_common, core, core_tests};
    use bcr_wdc_utils::{keys::test_utils as keys_test, signatures::test_utils as signatures_test};
    use bitcoin::{
        key::rand,
        secp256k1::{self as secp, schnorr},
    };

    fn random_cdk_pks(sz: usize) -> Vec<cashu::PublicKey> {
        std::iter::repeat_with(|| {
            cashu::PublicKey::from(bcr_common::core::generate_random_keypair().public_key())
        })
        .take(sz)
        .collect()
    }

    async fn init_surreal_db() -> impl Repository {
        let sdb = surrealdb::Surreal::<surrealdb::engine::any::Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        surreal::Repository::from_db(sdb)
    }
    fn init_memmap_db() -> impl Repository {
        inmemory::Repository::default()
    }

    //////////////////////////////////////////////////////////////////// KeysRepository

    #[tokio::test]
    async fn test_keysrepo_info() {
        let db = init_memmap_db();
        keysrepo_info(db).await;
        //
        let db = init_surreal_db().await;
        keysrepo_info(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_keysrepo_info_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        keysrepo_info(db).await;
    }
    async fn keysrepo_info(db: impl Repository) {
        let (kinfo, keyset) = core_tests::generate_random_ecash_keyset();
        let entry = keys_utils::MintKeysEntry {
            id: kinfo.id,
            unit: kinfo.unit.clone(),
            active: kinfo.active,
            valid_from: kinfo.valid_from,
            derivation_path: kinfo.derivation_path.clone(),
            derivation_path_index: kinfo.derivation_path_index,
            amounts: kinfo.amounts.clone(),
            input_fee_ppk: kinfo.input_fee_ppk,
            final_expiry: kinfo.final_expiry,
            keys: keyset.keys,
        };
        db.keys_store(entry).await.unwrap();
        let rinfo = db.keys_info(kinfo.id).await.unwrap().unwrap();
        assert_eq!(rinfo, kinfo);
    }

    #[tokio::test]
    async fn test_keysrepo_listinfo() {
        let db = init_memmap_db();
        keysrepo_list_info(db).await;
        //
        let db = init_surreal_db().await;
        keysrepo_list_info(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_keysrepo_listinfo_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        keysrepo_list_info(db).await;
    }
    async fn keysrepo_list_info(db: impl Repository) {
        let (mut info1, keyset1) = core_tests::generate_random_ecash_keyset();
        info1.unit = cashu::CurrencyUnit::Sat;
        info1.final_expiry = Some(10);
        let entry1 = keys_utils::MintKeysEntry {
            id: info1.id,
            unit: info1.unit,
            active: info1.active,
            valid_from: info1.valid_from,
            derivation_path: info1.derivation_path,
            derivation_path_index: info1.derivation_path_index,
            amounts: info1.amounts,
            input_fee_ppk: info1.input_fee_ppk,
            final_expiry: info1.final_expiry,
            keys: keyset1.keys,
        };
        db.keys_store(entry1).await.unwrap();
        let (mut info2, keyset2) = core_tests::generate_random_ecash_keyset();
        info2.unit = cashu::CurrencyUnit::Usd;
        info2.final_expiry = Some(20);
        let entry2 = keys_utils::MintKeysEntry {
            id: info2.id,
            unit: info2.unit,
            active: info2.active,
            valid_from: info2.valid_from,
            derivation_path: info2.derivation_path,
            derivation_path_index: info2.derivation_path_index,
            amounts: info2.amounts,
            input_fee_ppk: info2.input_fee_ppk,
            final_expiry: info2.final_expiry,
            keys: keyset2.keys,
        };
        db.keys_store(entry2).await.unwrap();
        let (mut info3, keyset3) = core_tests::generate_random_ecash_keyset();
        info3.unit = cashu::CurrencyUnit::Usd;
        info3.final_expiry = Some(30);
        let entry3 = keys_utils::MintKeysEntry {
            id: info3.id,
            unit: info3.unit,
            active: info3.active,
            valid_from: info3.valid_from,
            derivation_path: info3.derivation_path,
            derivation_path_index: info3.derivation_path_index,
            amounts: info3.amounts,
            input_fee_ppk: info3.input_fee_ppk,
            final_expiry: info3.final_expiry,
            keys: keyset3.keys,
        };
        db.keys_store(entry3).await.unwrap();

        let rinfos = db.keys_list_info(None, None, None).await.unwrap();
        assert_eq!(rinfos.len(), 3);

        let rinfos = db
            .keys_list_info(Some(cashu::CurrencyUnit::Sat), None, None)
            .await
            .unwrap();
        assert_eq!(rinfos.len(), 1);
        assert_eq!(rinfos[0].unit, cashu::CurrencyUnit::Sat);

        let rinfos = db
            .keys_list_info(Some(cashu::CurrencyUnit::Usd), Some(15), Some(25))
            .await
            .unwrap();
        assert_eq!(rinfos.len(), 1);
        assert_eq!(rinfos[0].final_expiry, Some(20));
        assert_eq!(rinfos[0].unit, cashu::CurrencyUnit::Usd);
    }

    #[tokio::test]
    async fn test_keysrepo_keyset() {
        let db = init_memmap_db();
        keysrepo_keyset_test(db).await;
        //
        let db = init_surreal_db().await;
        keysrepo_keyset_test(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_keysrepo_keyset_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        keysrepo_keyset_test(db).await;
    }
    async fn keysrepo_keyset_test(db: impl Repository) {
        let (kinfo, keyset) = core_tests::generate_random_ecash_keyset();
        let entry = keys_utils::MintKeysEntry {
            id: kinfo.id,
            unit: kinfo.unit,
            active: kinfo.active,
            valid_from: kinfo.valid_from,
            derivation_path: kinfo.derivation_path,
            derivation_path_index: kinfo.derivation_path_index,
            amounts: kinfo.amounts,
            input_fee_ppk: kinfo.input_fee_ppk,
            final_expiry: kinfo.final_expiry,
            keys: keyset.keys.clone(),
        };
        db.keys_store(entry.clone()).await.unwrap();
        let rkeys = db.keys_load(entry.id).await.unwrap().unwrap();
        assert_eq!(rkeys, keyset);
    }

    #[tokio::test]
    async fn test_keysrepo_infos_for_expiration_date() {
        let db = init_memmap_db();
        keysrepo_infos_for_expiration_date_test(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_keysrepo_infos_for_expiration_date_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        keysrepo_infos_for_expiration_date_test(db).await;
    }
    async fn keysrepo_infos_for_expiration_date_test(db: impl Repository) {
        let (mut info0, keyset0) = core_tests::generate_random_ecash_keyset();
        info0.final_expiry = Some(30);
        let keys0 = keys_utils::MintKeysEntry {
            id: info0.id,
            unit: info0.unit,
            active: info0.active,
            valid_from: info0.valid_from,
            derivation_path: info0.derivation_path,
            derivation_path_index: info0.derivation_path_index,
            amounts: info0.amounts,
            input_fee_ppk: info0.input_fee_ppk,
            final_expiry: info0.final_expiry,
            keys: keyset0.keys,
        };
        db.keys_store(keys0).await.unwrap();
        let (mut info1, keyset1) = core_tests::generate_random_ecash_keyset();
        info1.final_expiry = Some(10);
        let keys1 = keys_utils::MintKeysEntry {
            id: info1.id,
            unit: info1.unit,
            active: info1.active,
            valid_from: info1.valid_from,
            derivation_path: info1.derivation_path,
            derivation_path_index: info1.derivation_path_index,
            amounts: info1.amounts,
            input_fee_ppk: info1.input_fee_ppk,
            final_expiry: info1.final_expiry,
            keys: keyset1.keys,
        };
        db.keys_store(keys1).await.unwrap();
        let res = db.keys_infos_for_expiration_date(10).await.unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].final_expiry, Some(10));
        assert_eq!(res[1].final_expiry, Some(30));
        let res = db.keys_infos_for_expiration_date(20).await.unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].final_expiry, Some(30));
    }

    //////////////////////////////////////////////////////////////////// SignaturesRepository

    #[tokio::test]
    async fn test_signsrepo_store() {
        let db = init_memmap_db();
        signsrepo_store(db).await;
        //
        let db = init_surreal_db().await;
        signsrepo_store(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_signsrepo_store_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        signsrepo_store(db).await;
    }
    async fn signsrepo_store(db: impl Repository) {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts = [cashu::Amount::from(8u64)];
        let y = keys_test::publics()[0];
        let signature = signatures_test::generate_signatures(keyset.id, &amounts)[0].clone();
        db.signature_store(y, signature).await.unwrap();
    }

    #[tokio::test]
    async fn test_signsrepo_store_same_signature_twice() {
        let db = init_memmap_db();
        signsrepo_store_same_signature_twice(db).await;
        //
        let db = init_surreal_db().await;
        signsrepo_store_same_signature_twice(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_signsrepo_store_same_signature_twice_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        signsrepo_store_same_signature_twice(db).await;
    }
    async fn signsrepo_store_same_signature_twice(db: impl Repository) {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts = [cashu::Amount::from(8u64)];
        let y = keys_test::publics()[0];
        let signature = signatures_test::generate_signatures(keyset.id, &amounts)[0].clone();
        db.signature_store(y, signature.clone()).await.unwrap();
        let res = db.signature_store(y, signature).await;
        assert!(matches!(res, Err(Error::Conflict(_))));
    }

    /////////////////////////////////////////////////////////////////// ProofRepository
    #[tokio::test]
    async fn test_proofsrepo_insert() {
        let db = init_memmap_db();
        proofsrepo_insert(db).await;
        //
        let db = init_surreal_db().await;
        proofsrepo_insert(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_proofsrepo_insert_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        proofsrepo_insert(db).await;
    }
    async fn proofsrepo_insert(db: impl Repository) {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(
            &keyset,
            &[cashu::Amount::from(16_u64), cashu::Amount::from(8_u64)],
        );
        db.proofs_insert(proofs.clone()).await.unwrap();
        db.proofs_contains(proofs[0].y().unwrap())
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn test_proofsrepo_insert_double_spent_all() {
        let db = init_memmap_db();
        proofsrepo_insert_double_spent_all(db).await;
        //
        let db = init_surreal_db().await;
        proofsrepo_insert_double_spent_all(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_proofsrepo_insert_double_spent_all_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        proofsrepo_insert_double_spent_all(db).await;
    }
    async fn proofsrepo_insert_double_spent_all(db: impl Repository) {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(
            &keyset,
            &[cashu::Amount::from(16_u64), cashu::Amount::from(8_u64)],
        );
        db.proofs_insert(proofs.clone()).await.unwrap();
        let res = db.proofs_insert(proofs).await;
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::Conflict(_)));
    }

    #[tokio::test]
    async fn test_proofsrepo_insert_double_spent_partial() {
        let db = init_memmap_db();
        proofsrepo_insert_double_spent_partial(db).await;
        //
        let db = init_surreal_db().await;
        proofsrepo_insert_double_spent_partial(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_proofsrepo_insert_double_spent_partial_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        proofsrepo_insert_double_spent_partial(db).await;
    }
    async fn proofsrepo_insert_double_spent_partial(db: impl Repository) {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(
            &keyset,
            &[
                cashu::Amount::from(16_u64),
                cashu::Amount::from(8_u64),
                cashu::Amount::from(4_u64),
            ],
        );
        db.proofs_insert(proofs[0..2].to_vec()).await.unwrap();
        let res = db.proofs_insert(proofs[1..].to_vec()).await;
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::Conflict(_)));
    }

    #[tokio::test]
    async fn test_proofsrepo_insert_double_spent_partial_still_valid() {
        let db = init_memmap_db();
        proofsrepo_insert_double_spent_partial_still_valid(db).await;
        //
        let db = init_surreal_db().await;
        proofsrepo_insert_double_spent_partial_still_valid(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_proofsrepo_insert_double_spent_partial_still_valid_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        proofsrepo_insert_double_spent_partial_still_valid(db).await;
    }
    async fn proofsrepo_insert_double_spent_partial_still_valid(db: impl Repository) {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(
            &keyset,
            &[
                cashu::Amount::from(16_u64),
                cashu::Amount::from(8_u64),
                cashu::Amount::from(4_u64),
            ],
        );
        db.proofs_insert(proofs[0..2].to_vec()).await.unwrap();
        let res = db.proofs_insert(proofs[1..].to_vec()).await;
        assert!(res.is_err());
        db.proofs_insert(proofs[2..].to_vec()).await.unwrap();
    }

    #[tokio::test]
    async fn test_proofsrepo_insert_duplicate_in_batch() {
        let db = init_memmap_db();
        proofsrepo_insert_duplicate_in_batch(db).await;
        //
        let db = init_surreal_db().await;
        proofsrepo_insert_duplicate_in_batch(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_proofsrepo_insert_duplicate_in_batch_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        proofsrepo_insert_duplicate_in_batch(db).await;
    }
    async fn proofsrepo_insert_duplicate_in_batch(db: impl Repository) {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proof =
            core_tests::generate_random_ecash_proofs(&keyset, &[cashu::Amount::from(16_u64)])
                .pop()
                .unwrap();
        let y = proof.y().unwrap();
        let result = db.proofs_insert(vec![proof.clone(), proof]).await;
        assert!(matches!(result, Err(Error::Conflict(_))));
        assert!(db.proofs_contains(y).await.unwrap().is_none());
    }

    /////////////////////////////////////////////////////////////////// CommitmentRepository

    fn random_wallet_key() -> cashu::PublicKey {
        let pk = secp::generate_keypair(&mut rand::thread_rng()).1;
        cashu::PublicKey::from(pk)
    }

    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_sqlx_proofs_insert_consumes_deadline_reservations(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(
            &keyset,
            &[cashu::Amount::from(16_u64), cashu::Amount::from(8_u64)],
        );
        let reserved_y = proofs[0].y().unwrap();
        let fresh_y = proofs[1].y().unwrap();
        let deadline = TStamp::from_timestamp(100000, 0).unwrap();
        db.ys_store(vec![reserved_y], deadline).await.unwrap();

        db.proofs_insert(proofs).await.unwrap();

        assert!(db.proofs_contains(reserved_y).await.unwrap().is_some());
        assert!(db.proofs_contains(fresh_y).await.unwrap().is_some());
        assert_eq!(
            db.ys_contains(&[reserved_y, fresh_y]).await.unwrap(),
            vec![false, false]
        );
        db.ys_clean_expired(TStamp::from_timestamp(200000, 0).unwrap())
            .await
            .unwrap();
        assert!(db.proofs_contains(reserved_y).await.unwrap().is_some());
    }

    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_sqlx_proofs_insert_consumes_commitment_reservations(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proof =
            core_tests::generate_random_ecash_proofs(&keyset, &[cashu::Amount::from(16_u64)])
                .pop()
                .unwrap();
        let committed_y = proof.y().unwrap();
        let expiration = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = signatures_test::random_schnorr_signature();
        db.commitment_store(
            vec![committed_y],
            random_cdk_pks(1),
            expiration,
            random_wallet_key(),
            signature,
            [0_u8; 32],
            SignatureOwner::Unsigned,
        )
        .await
        .unwrap();

        db.proofs_insert(vec![proof]).await.unwrap();

        assert!(db.proofs_contains(committed_y).await.unwrap().is_some());
        assert!(!db.commitment_contains_inputs(&[committed_y]).await.unwrap());
        db.commitment_delete(signature).await.unwrap();
        assert!(db.proofs_contains(committed_y).await.unwrap().is_some());
    }

    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_sqlx_proof_spend_wins_concurrent_reservation(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proof =
            core_tests::generate_random_ecash_proofs(&keyset, &[cashu::Amount::from(16_u64)])
                .pop()
                .unwrap();
        let y = proof.y().unwrap();
        let deadline = TStamp::from_timestamp(100000, 0).unwrap();
        let reserve_db = db.clone();
        let spend_db = db.clone();

        let (reserve_result, spend_result) = tokio::join!(
            reserve_db.ys_store(vec![y], deadline),
            spend_db.proofs_insert(vec![proof]),
        );

        assert!(matches!(reserve_result, Ok(()) | Err(Error::Conflict(_))));
        assert!(spend_result.is_ok());
        assert!(db.proofs_contains(y).await.unwrap().is_some());
        assert_eq!(db.ys_contains(&[y]).await.unwrap(), vec![false]);
    }

    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_sqlx_commitment_store_rejects_spent_or_reserved_fingerprints(
        pool: ::sqlx::PgPool,
    ) {
        let db = sqlx::Repository::from_pool(pool);
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proof =
            core_tests::generate_random_ecash_proofs(&keyset, &[cashu::Amount::from(16_u64)])
                .pop()
                .unwrap();
        let spent_y = proof.y().unwrap();
        let reserved_y = random_cdk_pks(1).pop().unwrap();
        let fresh_y = random_cdk_pks(1).pop().unwrap();
        let expiration = TStamp::from_timestamp(100000, 0).unwrap();
        db.proofs_insert(vec![proof]).await.unwrap();
        db.ys_store(vec![reserved_y], expiration).await.unwrap();

        for unavailable_y in [spent_y, reserved_y] {
            let signature = signatures_test::random_schnorr_signature();
            let result = db
                .commitment_store(
                    vec![unavailable_y, fresh_y],
                    random_cdk_pks(1),
                    expiration,
                    random_wallet_key(),
                    signature,
                    [0_u8; 32],
                    SignatureOwner::Unsigned,
                )
                .await;

            assert!(matches!(result, Err(Error::Conflict(_))));
            assert!(matches!(
                db.commitment_load(&signature).await,
                Err(Error::ResourceNotFound(_))
            ));
            assert!(!db.commitment_contains_inputs(&[fresh_y]).await.unwrap());
        }
    }

    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_sqlx_ys_store_rejects_spent_or_committed_fingerprints(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proof =
            core_tests::generate_random_ecash_proofs(&keyset, &[cashu::Amount::from(16_u64)])
                .pop()
                .unwrap();
        let spent_y = proof.y().unwrap();
        let committed_y = random_cdk_pks(1).pop().unwrap();
        let fresh_y = random_cdk_pks(1).pop().unwrap();
        let deadline = TStamp::from_timestamp(100000, 0).unwrap();
        db.proofs_insert(vec![proof]).await.unwrap();
        db.commitment_store(
            vec![committed_y],
            random_cdk_pks(1),
            deadline,
            random_wallet_key(),
            signatures_test::random_schnorr_signature(),
            [0_u8; 32],
            SignatureOwner::Unsigned,
        )
        .await
        .unwrap();

        for unavailable_y in [spent_y, committed_y] {
            let result = db.ys_store(vec![unavailable_y, fresh_y], deadline).await;

            assert!(matches!(result, Err(Error::Conflict(_))));
            assert_eq!(
                db.ys_contains(&[unavailable_y, fresh_y]).await.unwrap(),
                vec![false, false]
            );
        }
    }

    #[tokio::test]
    async fn test_commitmentsrepo_store_duplicates() {
        let db = init_memmap_db();
        commitmentsrepo_store_duplicates(db).await;
        //
        let db = init_surreal_db().await;
        commitmentsrepo_store_duplicates(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_commitmentsrepo_store_duplicates_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        commitmentsrepo_store_duplicates(db).await;
    }
    async fn commitmentsrepo_store_duplicates(db: impl Repository) {
        let inputs = random_cdk_pks(5);
        let outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = signatures_test::random_schnorr_signature();
        db.commitment_store(
            inputs.clone(),
            outputs.clone(),
            tstamp,
            random_wallet_key(),
            signature,
            [0u8; 32],
            SignatureOwner::Unsigned,
        )
        .await
        .unwrap();
        let mut duplicated_inputs = random_cdk_pks(3);
        duplicated_inputs.push(inputs[0]);
        let mut duplicated_outputs = random_cdk_pks(3);
        let signature = signatures_test::random_schnorr_signature();
        let res = db
            .commitment_store(
                duplicated_inputs,
                outputs.clone(),
                tstamp,
                random_wallet_key(),
                signature,
                [0u8; 32],
                SignatureOwner::Unsigned,
            )
            .await;
        assert!(res.is_err());
        duplicated_outputs.push(outputs[0]);
        let signature = signatures_test::random_schnorr_signature();
        let res = db
            .commitment_store(
                inputs,
                duplicated_outputs,
                tstamp,
                random_wallet_key(),
                signature,
                [0u8; 32],
                SignatureOwner::Unsigned,
            )
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_commitmentsrepo_contains_inputs() {
        let db = init_memmap_db();
        commitmentsrepo_contains_inputs(db).await;
        //
        let db = init_surreal_db().await;
        commitmentsrepo_contains_inputs(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_commitmentsrepo_contains_inputs_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        commitmentsrepo_contains_inputs(db).await;
    }
    async fn commitmentsrepo_contains_inputs(db: impl Repository) {
        let inputs = random_cdk_pks(5);
        let outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = signatures_test::random_schnorr_signature();
        db.commitment_store(
            inputs.clone(),
            outputs.clone(),
            tstamp,
            random_wallet_key(),
            signature,
            [0u8; 32],
            SignatureOwner::Unsigned,
        )
        .await
        .unwrap();
        let mut tester = random_cdk_pks(2);
        let result = db.commitment_contains_inputs(&tester).await;
        assert!(!result.unwrap());
        tester.push(inputs[0]);
        let result = db.commitment_contains_inputs(&tester).await;
        assert!(result.unwrap());
        let result = db.commitment_contains_inputs(&inputs).await;
        assert!(result.unwrap());
        let result = db.commitment_contains_inputs(&outputs).await;
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_commitmentsrepo_contains_outputs() {
        let db = init_memmap_db();
        commitmentsrepo_contains_outputs(db).await;
        //
        let db = init_surreal_db().await;
        commitmentsrepo_contains_outputs(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_commitmentsrepo_contains_outputs_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        commitmentsrepo_contains_outputs(db).await;
    }
    async fn commitmentsrepo_contains_outputs(db: impl Repository) {
        let inputs = random_cdk_pks(5);
        let outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = signatures_test::random_schnorr_signature();
        db.commitment_store(
            inputs.clone(),
            outputs.clone(),
            tstamp,
            random_wallet_key(),
            signature,
            [0u8; 32],
            SignatureOwner::Unsigned,
        )
        .await
        .unwrap();
        let mut tester = random_cdk_pks(2);
        let result = db.commitment_contains_outputs(&tester).await;
        assert!(!result.unwrap());
        tester.push(outputs[0]);
        let result = db.commitment_contains_outputs(&tester).await;
        assert!(result.unwrap());
        let result = db.commitment_contains_outputs(&outputs).await;
        assert!(result.unwrap());
        let result = db.commitment_contains_outputs(&inputs).await;
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_commitmentsrepo_load() {
        let db = init_memmap_db();
        commitmentsrepo_load(db).await;
        //
        let db = init_surreal_db().await;
        commitmentsrepo_load(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_commitmentsrepo_load_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        commitmentsrepo_load(db).await;
    }
    async fn commitmentsrepo_load(db: impl Repository) {
        let mut inputs = random_cdk_pks(5);
        let mut outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = signatures_test::random_schnorr_signature();
        let fp_digest = [7u8; 32];
        db.commitment_store(
            inputs.clone(),
            outputs.clone(),
            tstamp,
            random_wallet_key(),
            signature,
            fp_digest,
            SignatureOwner::Beta,
        )
        .await
        .unwrap();
        let mut result = db.commitment_load(&signature).await.unwrap();
        result.inputs.sort();
        inputs.sort();
        assert_eq!(result.inputs, inputs);
        result.outputs.sort();
        outputs.sort();
        assert_eq!(result.outputs, outputs);
        assert_eq!(result.expiration, tstamp);
        assert_eq!(result.fp_digest, fp_digest);
        assert_eq!(result.signed, SignatureOwner::Beta);
    }

    #[tokio::test]
    async fn test_commitmentsrepo_store_duplicate_signature() {
        let db = init_memmap_db();
        commitmentsrepo_store_duplicate_signature(db).await;
        //
        let db = init_surreal_db().await;
        commitmentsrepo_store_duplicate_signature(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_commitmentsrepo_store_duplicate_signature_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        commitmentsrepo_store_duplicate_signature(db).await;
    }
    async fn commitmentsrepo_store_duplicate_signature(db: impl Repository) {
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = signatures_test::random_schnorr_signature();
        db.commitment_store(
            random_cdk_pks(5),
            random_cdk_pks(3),
            tstamp,
            random_wallet_key(),
            signature,
            [0u8; 32],
            SignatureOwner::Unsigned,
        )
        .await
        .unwrap();
        let result = db
            .commitment_store(
                random_cdk_pks(5),
                random_cdk_pks(3),
                tstamp,
                random_wallet_key(),
                signature,
                [1u8; 32],
                SignatureOwner::Alpha,
            )
            .await;
        assert!(matches!(result, Err(Error::Conflict(_))));
    }

    #[tokio::test]
    async fn test_commitmentsrepo_delete_releases_inputs_outputs() {
        let db = init_memmap_db();
        commitmentsrepo_delete_releases_inputs_outputs(db).await;
        //
        let db = init_surreal_db().await;
        commitmentsrepo_delete_releases_inputs_outputs(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_commitmentsrepo_delete_releases_inputs_outputs_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        commitmentsrepo_delete_releases_inputs_outputs(db).await;
    }
    async fn commitmentsrepo_delete_releases_inputs_outputs(db: impl Repository) {
        let inputs = random_cdk_pks(5);
        let outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = signatures_test::random_schnorr_signature();
        db.commitment_store(
            inputs.clone(),
            outputs.clone(),
            tstamp,
            random_wallet_key(),
            signature,
            [0u8; 32],
            SignatureOwner::Unsigned,
        )
        .await
        .unwrap();
        db.commitment_delete(signature).await.unwrap();
        assert!(!db.commitment_contains_inputs(&inputs).await.unwrap());
        assert!(!db.commitment_contains_outputs(&outputs).await.unwrap());
        db.commitment_store(
            inputs,
            outputs,
            tstamp,
            random_wallet_key(),
            signatures_test::random_schnorr_signature(),
            [0u8; 32],
            SignatureOwner::Unsigned,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_commitmentsrepo_clean_expired() {
        let db = init_memmap_db();
        commitmentsrepo_clean_expired(db).await;
        //
        let db = init_surreal_db().await;
        commitmentsrepo_clean_expired(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_commitmentsrepo_clean_expired_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        commitmentsrepo_clean_expired(db).await;
    }
    async fn commitmentsrepo_clean_expired(db: impl Repository) {
        let past_inputs = random_cdk_pks(5);
        let past_outputs = random_cdk_pks(3);
        let future_inputs = random_cdk_pks(5);
        let future_outputs = random_cdk_pks(3);
        let past = TStamp::from_timestamp(100000, 0).unwrap();
        let future = TStamp::from_timestamp(200000, 0).unwrap();
        db.commitment_store(
            past_inputs.clone(),
            past_outputs.clone(),
            past,
            random_wallet_key(),
            signatures_test::random_schnorr_signature(),
            [0u8; 32],
            SignatureOwner::Unsigned,
        )
        .await
        .unwrap();
        db.commitment_store(
            future_inputs.clone(),
            future_outputs.clone(),
            future,
            random_wallet_key(),
            signatures_test::random_schnorr_signature(),
            [0u8; 32],
            SignatureOwner::Unsigned,
        )
        .await
        .unwrap();
        db.commitment_clean_expired(TStamp::from_timestamp(150000, 0).unwrap())
            .await
            .unwrap();
        assert!(!db.commitment_contains_inputs(&past_inputs).await.unwrap());
        assert!(!db.commitment_contains_outputs(&past_outputs).await.unwrap());
        assert!(db.commitment_contains_inputs(&future_inputs).await.unwrap());
        assert!(db
            .commitment_contains_outputs(&future_outputs)
            .await
            .unwrap());
    }

    /////////////////////////////////////////////////////////////////// ReservedYsRepository

    #[tokio::test]
    async fn test_reservedysrepo_contains() {
        let db = init_memmap_db();
        reservedysrepo_contains(db).await;
        //
        let db = init_surreal_db().await;
        reservedysrepo_contains(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_reservedysrepo_contains_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        reservedysrepo_contains(db).await;
    }
    async fn reservedysrepo_contains(db: impl Repository) {
        let inputs = random_cdk_pks(5);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        db.ys_store(inputs.clone(), tstamp).await.unwrap();
        let mut tester = random_cdk_pks(2);
        let result = db.ys_contains(&tester).await.unwrap();
        assert!(result.iter().all(|r| !r));
        tester.push(inputs[0]);
        let result = db.ys_contains(&tester).await.unwrap();
        assert!(result[0..2].iter().all(|r| !r));
        assert!(result[2]);
    }

    #[tokio::test]
    async fn test_reservedysrepo_clean_expired() {
        let db = init_memmap_db();
        reservedysrepo_clean_expired(db).await;
        //
        let db = init_surreal_db().await;
        reservedysrepo_clean_expired(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_reservedysrepo_clean_expired_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        reservedysrepo_clean_expired(db).await;
    }
    async fn reservedysrepo_clean_expired(db: impl Repository) {
        let inputs = random_cdk_pks(5);
        let past = TStamp::from_timestamp(100000, 0).unwrap();
        let future = TStamp::from_timestamp(200000, 0).unwrap();
        db.ys_store(inputs.clone(), past).await.unwrap();
        db.ys_clean_expired(TStamp::from_timestamp(150000, 0).unwrap())
            .await
            .unwrap();
        let result = db.ys_contains(&inputs).await.unwrap();
        assert!(result.iter().all(|r| !r));
        db.ys_store(inputs.clone(), future).await.unwrap();
        db.ys_clean_expired(TStamp::from_timestamp(150000, 0).unwrap())
            .await
            .unwrap();
        let result = db.ys_contains(&inputs).await.unwrap();
        assert!(result.iter().all(|r| *r));
    }

    #[tokio::test]
    async fn test_reservedysrepo_store_conflict() {
        let db = init_memmap_db();
        reservedysrepo_store_conflict(db).await;
        //
        let db = init_surreal_db().await;
        reservedysrepo_store_conflict(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_reservedysrepo_store_conflict_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        reservedysrepo_store_conflict(db).await;
    }
    async fn reservedysrepo_store_conflict(db: impl Repository) {
        let inputs = random_cdk_pks(2);
        let fresh_input = random_cdk_pks(1).pop().unwrap();
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        db.ys_store(inputs.clone(), tstamp).await.unwrap();
        let result = db.ys_store(vec![inputs[0], fresh_input], tstamp).await;
        assert!(matches!(result, Err(Error::Conflict(_))));
        let result = db
            .ys_contains(&[inputs[0], inputs[1], fresh_input])
            .await
            .unwrap();
        assert_eq!(result, vec![true, true, false]);
    }

    #[tokio::test]
    async fn test_reservedysrepo_store_duplicate_batch() {
        let db = init_memmap_db();
        reservedysrepo_store_duplicate_batch(db).await;
        //
        let db = init_surreal_db().await;
        reservedysrepo_store_duplicate_batch(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_reservedysrepo_store_duplicate_batch_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        reservedysrepo_store_duplicate_batch(db).await;
    }
    async fn reservedysrepo_store_duplicate_batch(db: impl Repository) {
        let input = random_cdk_pks(1).pop().unwrap();
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let result = db.ys_store(vec![input, input], tstamp).await;
        assert!(matches!(result, Err(Error::Conflict(_))));
        let result = db.ys_contains(&[input]).await.unwrap();
        assert_eq!(result, vec![false]);
    }

    #[tokio::test]
    async fn test_reservedysrepo_store_after_clean_expired() {
        let db = init_memmap_db();
        reservedysrepo_store_after_clean_expired(db).await;
        //
        let db = init_surreal_db().await;
        reservedysrepo_store_after_clean_expired(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_reservedysrepo_store_after_clean_expired_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::Repository::from_pool(pool);
        reservedysrepo_store_after_clean_expired(db).await;
    }
    async fn reservedysrepo_store_after_clean_expired(db: impl Repository) {
        let input = random_cdk_pks(1).pop().unwrap();
        let past = TStamp::from_timestamp(100000, 0).unwrap();
        let future = TStamp::from_timestamp(200000, 0).unwrap();
        db.ys_store(vec![input], past).await.unwrap();
        let result = db.ys_store(vec![input], future).await;
        assert!(matches!(result, Err(Error::Conflict(_))));
        db.ys_clean_expired(TStamp::from_timestamp(150000, 0).unwrap())
            .await
            .unwrap();
        db.ys_store(vec![input], future).await.unwrap();
        let result = db.ys_contains(&[input]).await.unwrap();
        assert_eq!(result, vec![true]);
    }

    fn swap_finalization_fixture(
        amounts: &[cashu::Amount],
    ) -> (
        Vec<cashu::Proof>,
        Vec<cashu::BlindedMessage>,
        Vec<StoredSignature>,
        schnorr::Signature,
    ) {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(&keyset, amounts);
        let blinds: Vec<cashu::BlindedMessage> =
            signatures_test::generate_blinds(keyset.id, amounts)
                .into_iter()
                .map(|b| b.0)
                .collect();
        let c_keyset = cdk_common::MintKeySet::from(keyset.clone());
        let signatures = blinds
            .iter()
            .map(|b| {
                let signature = bcr_common::core::signature::sign_ecash(&c_keyset, b).unwrap();
                StoredSignature {
                    y: b.blinded_secret,
                    signature,
                }
            })
            .collect();
        let commitment = schnorr::Signature::from_slice(&[42u8; 64]).unwrap();
        (proofs, blinds, signatures, commitment)
    }

    async fn store_swap_commitment(
        db: &impl Repository,
        proofs: Vec<cashu::Proof>,
        blinds: Vec<cashu::BlindedMessage>,
        commitment: schnorr::Signature,
    ) {
        let ys = proofs.ys().unwrap();
        let bs = blinds.iter().map(|b| b.blinded_secret).collect();
        let kp = core::generate_random_keypair();
        db.commitment_store(
            ys,
            bs,
            chrono::Utc::now() + chrono::Duration::minutes(1),
            kp.public_key().into(),
            commitment,
            [0u8; 32],
            SignatureOwner::Unsigned,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_swap_finalize_is_atomic() {
        swap_finalize_is_atomic(init_memmap_db()).await;
        swap_finalize_is_atomic(init_surreal_db().await).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_swap_finalize_is_atomic_sqlx(pool: ::sqlx::PgPool) {
        swap_finalize_is_atomic(sqlx::Repository::from_pool(pool)).await;
    }
    async fn swap_finalize_is_atomic(db: impl Repository) {
        let (proofs, blinds, stored_signatures, commitment) =
            swap_finalization_fixture(&[cashu::Amount::from(16_u64)]);
        store_swap_commitment(&db, proofs.clone(), blinds.clone(), commitment).await;
        db.swap_finalize(proofs.clone(), stored_signatures.clone(), commitment)
            .await
            .unwrap();
        let result = db
            .proofs_contains(proofs.first().unwrap().y().unwrap())
            .await
            .unwrap();
        assert!(result.is_some());
        let result = db.signature_load(&blinds[0]).await.unwrap();
        let expected = Some(stored_signatures[0].signature.clone());
        assert_eq!(result, expected);
        let result = db.commitment_load(&commitment).await;
        assert!(matches!(result, Err(Error::ResourceNotFound(_))));
    }

    #[tokio::test]
    async fn test_swap_finalize_rolls_back_on_spent_proof() {
        swap_finalize_rolls_back_on_spent_proof(init_memmap_db()).await;
        swap_finalize_rolls_back_on_spent_proof(init_surreal_db().await).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_swap_finalize_rolls_back_on_spent_proof_sqlx(pool: ::sqlx::PgPool) {
        swap_finalize_rolls_back_on_spent_proof(sqlx::Repository::from_pool(pool)).await;
    }
    async fn swap_finalize_rolls_back_on_spent_proof(db: impl Repository) {
        let (proofs, blinds, stored_signatures, commitment) =
            swap_finalization_fixture(&[cashu::Amount::from(16_u64)]);
        store_swap_commitment(&db, proofs.clone(), blinds.clone(), commitment).await;
        db.proofs_insert(proofs.clone()).await.unwrap();
        let result = db
            .swap_finalize(proofs.clone(), stored_signatures.clone(), commitment)
            .await;
        assert!(result.is_err());
        let result = db.signature_load(&blinds[0]).await.unwrap();
        assert_eq!(result, None);
        let result = db.commitment_load(&commitment).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_swap_finalize_rolls_back_on_existing_signature() {
        swap_finalize_rolls_back_on_existing_signature(init_memmap_db()).await;
        swap_finalize_rolls_back_on_existing_signature(init_surreal_db().await).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_swap_finalize_rolls_back_on_existing_signature_sqlx(pool: ::sqlx::PgPool) {
        swap_finalize_rolls_back_on_existing_signature(sqlx::Repository::from_pool(pool)).await;
    }
    async fn swap_finalize_rolls_back_on_existing_signature(db: impl Repository) {
        let (proofs, blinds, stored_signatures, commitment) =
            swap_finalization_fixture(&[cashu::Amount::from(16_u64)]);
        store_swap_commitment(&db, proofs.clone(), blinds.clone(), commitment).await;
        db.signature_store(
            stored_signatures[0].y,
            stored_signatures[0].signature.clone(),
        )
        .await
        .unwrap();
        let result = db
            .swap_finalize(proofs.clone(), stored_signatures, commitment)
            .await;
        assert!(result.is_err());
        let result = db.proofs_contains(proofs[0].y().unwrap()).await.unwrap();
        assert!(result.is_none());
        let result = db.commitment_load(&commitment).await;
        assert!(result.is_ok());
    }
}
