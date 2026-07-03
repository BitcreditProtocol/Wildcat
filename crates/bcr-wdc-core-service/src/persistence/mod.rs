// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{cashu, cdk_common::mint::MintKeySetInfo};
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
pub trait KeysRepository: Send + Sync {
    async fn store(&self, keys: keys_utils::KeysetEntry) -> Result<()>;
    async fn info(&self, id: cashu::Id) -> Result<Option<MintKeySetInfo>>;
    async fn keyset(&self, id: cashu::Id) -> Result<Option<cashu::MintKeySet>>;
    async fn list_info(
        &self,
        currency: Option<cashu::CurrencyUnit>,
        min_expiration_tstamp: Option<u64>,
        max_expiration_tstamp: Option<u64>,
    ) -> Result<Vec<MintKeySetInfo>>;
    async fn list_keyset(&self) -> Result<Vec<cashu::MintKeySet>>;
    async fn update_info(&self, info: MintKeySetInfo) -> Result<()>;
    async fn infos_for_expiration_date(&self, expire: u64) -> Result<Vec<MintKeySetInfo>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait SignaturesRepository: Send + Sync {
    async fn store(&self, y: cashu::PublicKey, signature: cashu::BlindSignature) -> Result<()>;
    async fn load(&self, blind: &cashu::BlindedMessage) -> Result<Option<cashu::BlindSignature>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ProofRepository: Send + Sync {
    /// WARNING: this method should do strict insert.
    /// i.e. it should fail if any of the proofs is already present in the DB
    /// in case of failure, the DB should be in the same state as before the call
    async fn insert(&self, tokens: Vec<cashu::Proof>) -> Result<()>;
    async fn remove(&self, tokens: &[cashu::PublicKey]) -> Result<()>;
    async fn contains(&self, y: cashu::PublicKey) -> Result<Option<cashu::ProofState>>;
}

#[derive(Debug, Clone, Copy)]
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

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait CommitmentRepository: Send + Sync {
    async fn store(
        &self,
        inputs: Vec<cashu::PublicKey>,
        outputs: Vec<cashu::PublicKey>,
        expiration: TStamp,
        wallet_key: cashu::PublicKey,
        commitment: schnorr::Signature,
        fp_digest: [u8; 32],
        signed: SignatureOwner,
    ) -> Result<()>;
    async fn load(&self, signature: &schnorr::Signature) -> Result<StoredCommitment>;
    async fn contains_inputs(&self, inputs: &[cashu::PublicKey]) -> Result<bool>;
    async fn contains_outputs(&self, outputs: &[cashu::PublicKey]) -> Result<bool>;
    async fn delete(&self, commitment: schnorr::Signature) -> Result<()>;
    async fn clean_expired(&self, now: TStamp) -> Result<()>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ReservedYsRepository: Send + Sync {
    async fn store(&self, inputs: Vec<cashu::PublicKey>, deadline: TStamp) -> Result<()>;
    async fn contains(&self, inputs: &[cashu::PublicKey]) -> Result<Vec<bool>>;
    async fn clean_expired(&self, now: TStamp) -> Result<()>;
    // no need to delete as inputs can only end up being burnt, and they will appear as spent in
    // ProofRepository, or they will be cleaned up after the deadline by clean_expired
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use bcr_common::core_tests;
    use bcr_wdc_utils::{keys::test_utils as keys_test, signatures::test_utils as signatures_test};
    use bitcoin::{key::rand, secp256k1 as secp};

    fn random_cdk_pks(sz: usize) -> Vec<cashu::PublicKey> {
        std::iter::repeat_with(|| {
            cashu::PublicKey::from(bcr_common::core::generate_random_keypair().public_key())
        })
        .take(sz)
        .collect()
    }
    //////////////////////////////////////////////////////////////////// KeysRepository
    async fn init_surreal_keys_db() -> impl KeysRepository {
        let sdb = surrealdb::Surreal::<surrealdb::engine::any::Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        surreal::DBKeys { db: sdb }
    }
    fn init_memmap_keys_db() -> impl KeysRepository {
        inmemory::KeyMap::default()
    }

    #[tokio::test]
    async fn test_keysrepo_info() {
        let db = init_memmap_keys_db();
        keysrepo_info(db).await;
        //
        let db = init_surreal_keys_db().await;
        keysrepo_info(db).await;
    }
    async fn keysrepo_info(db: impl KeysRepository) {
        let entry = core_tests::generate_random_ecash_keyset();
        let kinfo = entry.0.clone();
        db.store(entry).await.unwrap();
        let rinfo = db.info(kinfo.id).await.unwrap().unwrap();
        assert_eq!(rinfo, kinfo);
    }

    #[tokio::test]
    async fn test_keysrepo_listinfo() {
        let db = init_memmap_keys_db();
        keysrepo_list_info(db).await;
        //
        let db = init_surreal_keys_db().await;
        keysrepo_list_info(db).await;
    }
    async fn keysrepo_list_info(db: impl KeysRepository) {
        let entry1 = core_tests::generate_random_ecash_keyset();
        db.store(entry1).await.unwrap();
        let entry2 = core_tests::generate_random_ecash_keyset();
        db.store(entry2).await.unwrap();
        let rinfos = db.list_info(None, None, None).await.unwrap();
        assert_eq!(rinfos.len(), 2);
    }

    #[tokio::test]
    async fn test_keysrepo_listinfo_with_unit() {
        let db = init_memmap_keys_db();
        keysrepo_list_info_with_unit(db).await;
        //
        let db = init_surreal_keys_db().await;
        keysrepo_list_info_with_unit(db).await;
    }
    async fn keysrepo_list_info_with_unit(db: impl KeysRepository) {
        let mut entry1 = core_tests::generate_random_ecash_keyset();
        entry1.0.unit = cashu::CurrencyUnit::Sat;
        entry1.0.final_expiry = Some(10);
        db.store(entry1).await.unwrap();
        let mut entry2 = core_tests::generate_random_ecash_keyset();
        entry2.0.unit = cashu::CurrencyUnit::Usd;
        db.store(entry2).await.unwrap();
        let rinfos = db
            .list_info(Some(cashu::CurrencyUnit::Sat), None, None)
            .await
            .unwrap();
        assert_eq!(rinfos.len(), 1);
        assert_eq!(rinfos[0].unit, cashu::CurrencyUnit::Sat);
    }
    #[tokio::test]
    async fn test_keysrepo_listinfo_with_expiration() {
        let db = init_memmap_keys_db();
        keysrepo_list_info_with_min_expiration(db).await;
        //
        let db = init_surreal_keys_db().await;
        keysrepo_list_info_with_min_expiration(db).await;
    }
    async fn keysrepo_list_info_with_min_expiration(db: impl KeysRepository) {
        let mut entry1 = core_tests::generate_random_ecash_keyset();
        entry1.0.final_expiry = Some(10);
        db.store(entry1).await.unwrap();
        let mut entry2 = core_tests::generate_random_ecash_keyset();
        entry2.0.final_expiry = Some(20);
        db.store(entry2).await.unwrap();
        let rinfos = db.list_info(None, None, None).await.unwrap();
        assert_eq!(rinfos.len(), 2);
        let rinfos = db.list_info(None, Some(15), None).await.unwrap();
        assert_eq!(rinfos.len(), 1);
        assert_eq!(rinfos[0].final_expiry, Some(20));
    }

    #[tokio::test]
    async fn test_keysrepo_listinfo_with_max_expiration() {
        let db = init_memmap_keys_db();
        keysrepo_list_info_with_max_expiration(db).await;
        //
        let db = init_surreal_keys_db().await;
        keysrepo_list_info_with_max_expiration(db).await;
    }
    async fn keysrepo_list_info_with_max_expiration(db: impl KeysRepository) {
        let mut entry1 = core_tests::generate_random_ecash_keyset();
        entry1.0.final_expiry = Some(10);
        db.store(entry1).await.unwrap();
        let mut entry2 = core_tests::generate_random_ecash_keyset();
        entry2.0.final_expiry = Some(20);
        db.store(entry2).await.unwrap();
        let rinfos = db.list_info(None, None, None).await.unwrap();
        assert_eq!(rinfos.len(), 2);
        let rinfos = db.list_info(None, None, Some(15)).await.unwrap();
        assert_eq!(rinfos.len(), 1);
        assert_eq!(rinfos[0].final_expiry, Some(10));
    }

    #[tokio::test]
    async fn test_keysrepo_keyset() {
        let db = init_memmap_keys_db();
        keysrepo_keyset_test(db).await;
        //
        let db = init_surreal_keys_db().await;
        keysrepo_keyset_test(db).await;
    }
    async fn keysrepo_keyset_test(db: impl KeysRepository) {
        let entry = core_tests::generate_random_ecash_keyset();
        db.store(entry.clone()).await.unwrap();
        let rkeys = db.keyset(entry.0.id).await.unwrap().unwrap();
        assert_eq!(rkeys, entry.1);
    }

    #[tokio::test]
    async fn test_keysrepo_list_keyset() {
        let db = init_memmap_keys_db();
        keysrepo_list_keyset_test(db).await;
        //
        let db = init_surreal_keys_db().await;
        keysrepo_list_keyset_test(db).await;
    }
    async fn keysrepo_list_keyset_test(db: impl KeysRepository) {
        let entry1 = core_tests::generate_random_ecash_keyset();
        db.store(entry1).await.unwrap();
        let entry2 = core_tests::generate_random_ecash_keyset();
        db.store(entry2).await.unwrap();
        let rkeys = db.list_keyset().await.unwrap();
        assert_eq!(rkeys.len(), 2);
    }

    #[tokio::test]
    async fn test_keysrepo_update_info() {
        let db = init_memmap_keys_db();
        keysrepo_update_info_test(db).await;
        //
        let db = init_surreal_keys_db().await;
        keysrepo_update_info_test(db).await;
    }
    async fn keysrepo_update_info_test(db: impl KeysRepository) {
        let entry = core_tests::generate_random_ecash_keyset();
        let (mut info, _) = entry.clone();
        db.store(entry).await.unwrap();
        info.active = false;
        db.update_info(info.clone()).await.unwrap();
        let updated_info = db.info(info.id).await.unwrap().unwrap();
        assert!(!updated_info.active);
    }

    #[tokio::test]
    async fn test_keysrepo_update_info_kid_not_present() {
        let db = init_memmap_keys_db();
        keysrepo_update_info_kid_not_present_test(db).await;
        //
        let db = init_surreal_keys_db().await;
        keysrepo_update_info_kid_not_present_test(db).await;
    }
    async fn keysrepo_update_info_kid_not_present_test(db: impl KeysRepository) {
        let (info, _) = core_tests::generate_random_ecash_keyset();
        let res = db.update_info(info).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn test_keysrepo_infos_for_expiration_date() {
        let db = init_memmap_keys_db();
        keysrepo_infos_for_expiration_date_test(db).await;
    }
    async fn keysrepo_infos_for_expiration_date_test(db: impl KeysRepository) {
        let mut keys0 = core_tests::generate_random_ecash_keyset();
        keys0.0.final_expiry = Some(30);
        db.store(keys0).await.unwrap();
        let mut keys1 = core_tests::generate_random_ecash_keyset();
        keys1.0.final_expiry = Some(10);
        db.store(keys1).await.unwrap();
        let res = db.infos_for_expiration_date(10).await.unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].final_expiry, Some(10));
        assert_eq!(res[1].final_expiry, Some(30));
        let res = db.infos_for_expiration_date(20).await.unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].final_expiry, Some(30));
    }

    //////////////////////////////////////////////////////////////////// SignaturesRepository
    async fn init_surreal_signatures_db() -> impl SignaturesRepository {
        let sdb = surrealdb::Surreal::<surrealdb::engine::any::Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        surreal::DBSignatures { db: sdb }
    }
    fn init_memmap_signatures_db() -> impl SignaturesRepository {
        inmemory::SignatureMap::default()
    }

    #[tokio::test]
    async fn test_signsrepo_store() {
        let db = init_memmap_signatures_db();
        signsrepo_store(db).await;
        //
        let db = init_surreal_signatures_db().await;
        signsrepo_store(db).await;
    }
    #[::sqlx::test]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_signsrepo_store_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBSignatures::from_pool(pool);
        signsrepo_store(db).await;
    }
    async fn signsrepo_store(db: impl SignaturesRepository) {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts = [cashu::Amount::from(8u64)];
        let y = keys_test::publics()[0];
        let signature = signatures_test::generate_signatures(&keyset, &amounts)[0].clone();
        db.store(y, signature).await.unwrap();
    }

    #[tokio::test]
    async fn test_signsrepo_store_same_signature_twice() {
        let db = init_memmap_signatures_db();
        signsrepo_store_same_signature_twice(db).await;
        //
        let db = init_surreal_signatures_db().await;
        signsrepo_store_same_signature_twice(db).await;
    }
    #[::sqlx::test]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_signsrepo_store_same_signature_twice_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBSignatures::from_pool(pool);
        signsrepo_store_same_signature_twice(db).await;
    }
    async fn signsrepo_store_same_signature_twice(db: impl SignaturesRepository) {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts = [cashu::Amount::from(8u64)];
        let y = keys_test::publics()[0];
        let signature = signatures_test::generate_signatures(&keyset, &amounts)[0].clone();
        db.store(y, signature.clone()).await.unwrap();
        let res = db.store(y, signature).await;
        assert!(matches!(res, Err(Error::Conflict(_))));
    }

    /////////////////////////////////////////////////////////////////// ProofRepository
    async fn init_surreal_proofs_db() -> impl ProofRepository {
        let sdb = surrealdb::Surreal::<surrealdb::engine::any::Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        surreal::DBProofs { db: sdb }
    }
    async fn init_proofs_mem_db() -> impl ProofRepository {
        inmemory::ProofMap::default()
    }

    #[tokio::test]
    async fn test_proofsrepo_insert() {
        let db = init_proofs_mem_db().await;
        proofsrepo_insert(db).await;
        //
        let db = init_surreal_proofs_db().await;
        proofsrepo_insert(db).await;
    }
    #[::sqlx::test]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_proofsrepo_insert_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBProofs::from_pool(pool);
        proofsrepo_insert(db).await;
    }
    async fn proofsrepo_insert(db: impl ProofRepository) {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(
            &keyset,
            &[cashu::Amount::from(16_u64), cashu::Amount::from(8_u64)],
        );
        db.insert(proofs.clone()).await.unwrap();
        db.contains(proofs[0].y().unwrap()).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn test_proofsrepo_insert_double_spent_all() {
        let db = init_proofs_mem_db().await;
        proofsrepo_insert_double_spent_all(db).await;
        //
        let db = init_surreal_proofs_db().await;
        proofsrepo_insert_double_spent_all(db).await;
    }
    #[::sqlx::test]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_proofsrepo_insert_double_spent_all_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBProofs::from_pool(pool);
        proofsrepo_insert_double_spent_all(db).await;
    }
    async fn proofsrepo_insert_double_spent_all(db: impl ProofRepository) {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(
            &keyset,
            &[cashu::Amount::from(16_u64), cashu::Amount::from(8_u64)],
        );
        db.insert(proofs.clone()).await.unwrap();
        let res = db.insert(proofs).await;
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::InvalidInput(_)));
    }

    #[tokio::test]
    async fn test_proofsrepo_insert_double_spent_partial() {
        let db = init_proofs_mem_db().await;
        proofsrepo_insert_double_spent_partial(db).await;
        //
        let db = init_surreal_proofs_db().await;
        proofsrepo_insert_double_spent_partial(db).await;
    }
    #[::sqlx::test]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_proofsrepo_insert_double_spent_partial_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBProofs::from_pool(pool);
        proofsrepo_insert_double_spent_partial(db).await;
    }
    async fn proofsrepo_insert_double_spent_partial(db: impl ProofRepository) {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(
            &keyset,
            &[
                cashu::Amount::from(16_u64),
                cashu::Amount::from(8_u64),
                cashu::Amount::from(4_u64),
            ],
        );
        db.insert(proofs[0..2].to_vec()).await.unwrap();
        let res = db.insert(proofs[1..].to_vec()).await;
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), Error::InvalidInput(_)));
    }

    #[tokio::test]
    async fn test_proofsrepo_insert_double_spent_partial_still_valid() {
        let db = init_proofs_mem_db().await;
        proofsrepo_insert_double_spent_partial_still_valid(db).await;
        //
        let db = init_surreal_proofs_db().await;
        proofsrepo_insert_double_spent_partial_still_valid(db).await;
    }
    #[::sqlx::test]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_proofsrepo_insert_double_spent_partial_still_valid_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBProofs::from_pool(pool);
        proofsrepo_insert_double_spent_partial_still_valid(db).await;
    }
    async fn proofsrepo_insert_double_spent_partial_still_valid(db: impl ProofRepository) {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(
            &keyset,
            &[
                cashu::Amount::from(16_u64),
                cashu::Amount::from(8_u64),
                cashu::Amount::from(4_u64),
            ],
        );
        db.insert(proofs[0..2].to_vec()).await.unwrap();
        let res = db.insert(proofs[1..].to_vec()).await;
        assert!(res.is_err());
        db.insert(proofs[2..].to_vec()).await.unwrap();
    }

    /////////////////////////////////////////////////////////////////// CommitmentRepository
    async fn init_surreal_commitments_db() -> impl CommitmentRepository {
        let sdb = surrealdb::Surreal::<surrealdb::engine::any::Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        surreal::DBCommitments { db: sdb }
    }
    fn init_memmap_commitments_db() -> impl CommitmentRepository {
        inmemory::CommitmentMap::default()
    }

    fn random_wallet_key() -> cashu::PublicKey {
        let pk = secp::generate_keypair(&mut rand::thread_rng()).1;
        cashu::PublicKey::from(pk)
    }

    #[tokio::test]
    async fn test_commitmentsrepo_store_duplicates() {
        let db = init_memmap_commitments_db();
        commitmentsrepo_store_duplicates(db).await;
        //
        let db = init_surreal_commitments_db().await;
        commitmentsrepo_store_duplicates(db).await;
    }
    async fn commitmentsrepo_store_duplicates(db: impl CommitmentRepository) {
        let inputs = random_cdk_pks(5);
        let outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = signatures_test::random_schnorr_signature();
        db.store(
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
            .store(
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
            .store(
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
        let db = init_memmap_commitments_db();
        commitmentsrepo_contains_inputs(db).await;
        //
        let db = init_surreal_commitments_db().await;
        commitmentsrepo_contains_inputs(db).await;
    }
    async fn commitmentsrepo_contains_inputs(db: impl CommitmentRepository) {
        let inputs = random_cdk_pks(5);
        let outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = signatures_test::random_schnorr_signature();
        db.store(
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
    async fn test_commitmentsrepo_contains_outputs() {
        let db = init_memmap_commitments_db();
        commitmentsrepo_contains_outputs(db).await;
        //
        let db = init_surreal_commitments_db().await;
        commitmentsrepo_contains_outputs(db).await;
    }
    async fn commitmentsrepo_contains_outputs(db: impl CommitmentRepository) {
        let inputs = random_cdk_pks(5);
        let outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = signatures_test::random_schnorr_signature();
        db.store(
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
    async fn test_commitmentsrepo_load() {
        let db = init_memmap_commitments_db();
        commitmentsrepo_load(db).await;
        //
        let db = init_surreal_commitments_db().await;
        commitmentsrepo_load(db).await;
    }
    async fn commitmentsrepo_load(db: impl CommitmentRepository) {
        let mut inputs = random_cdk_pks(5);
        let mut outputs = random_cdk_pks(3);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        let signature = signatures_test::random_schnorr_signature();
        db.store(
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
        let mut result = db.load(&signature).await.unwrap();
        result.inputs.sort();
        inputs.sort();
        assert_eq!(result.inputs, inputs);
        result.outputs.sort();
        outputs.sort();
        assert_eq!(result.outputs, outputs);
        assert_eq!(result.expiration, tstamp)
    }

    /////////////////////////////////////////////////////////////////// ReservedYsRepository
    async fn init_surreal_reserved_ys_db() -> impl ReservedYsRepository {
        let sdb = surrealdb::Surreal::<surrealdb::engine::any::Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        surreal::DBReservedYs { db: sdb }
    }
    fn init_memmap_reserved_ys_db() -> impl ReservedYsRepository {
        inmemory::ReservedYsMap::default()
    }

    #[tokio::test]
    async fn test_reservedysrepo_contains() {
        let db = init_memmap_reserved_ys_db();
        reservedysrepo_contains(db).await;
        //
        let db = init_surreal_reserved_ys_db().await;
        reservedysrepo_contains(db).await;
    }
    async fn reservedysrepo_contains(db: impl ReservedYsRepository) {
        let inputs = random_cdk_pks(5);
        let tstamp = TStamp::from_timestamp(100000, 0).unwrap();
        db.store(inputs.clone(), tstamp).await.unwrap();
        let mut tester = random_cdk_pks(2);
        let result = db.contains(&tester).await.unwrap();
        assert!(result.iter().all(|r| !r));
        tester.push(inputs[0]);
        let result = db.contains(&tester).await.unwrap();
        assert!(result[0..2].iter().all(|r| !r));
        assert!(result[2]);
    }

    #[tokio::test]
    async fn test_reservedysrepo_clean_expired() {
        let db = init_memmap_reserved_ys_db();
        reservedysrepo_clean_expired(db).await;
        //
        let db = init_surreal_reserved_ys_db().await;
        reservedysrepo_clean_expired(db).await;
    }
    async fn reservedysrepo_clean_expired(db: impl ReservedYsRepository) {
        let inputs = random_cdk_pks(5);
        let past = TStamp::from_timestamp(100000, 0).unwrap();
        let future = TStamp::from_timestamp(200000, 0).unwrap();
        db.store(inputs.clone(), past).await.unwrap();
        db.clean_expired(TStamp::from_timestamp(150000, 0).unwrap())
            .await
            .unwrap();
        let result = db.contains(&inputs).await.unwrap();
        assert!(result.iter().all(|r| !r));
        db.store(inputs.clone(), future).await.unwrap();
        db.clean_expired(TStamp::from_timestamp(150000, 0).unwrap())
            .await
            .unwrap();
        let result = db.contains(&inputs).await.unwrap();
        assert!(result.iter().all(|r| *r));
    }
}
