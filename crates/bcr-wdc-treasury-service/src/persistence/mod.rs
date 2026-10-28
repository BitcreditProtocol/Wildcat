// ----- standard library imports
// ----- extra library imports
// ----- local imports
// ----- local modules
pub mod inmemory;
pub mod sqlx;
pub mod surreal;

// ----- end imports

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ebill, error::Error, foreign, onchain, vault};
    use bcr_common::{cashu, core, core_tests, wire::keys as wire_keys};
    use bcr_wdc_utils::signatures::test_utils as signature_tests;
    use std::str::FromStr;
    use uuid::Uuid;

    //////////////////////////////////////////////////////////////////// foreign::OnlineRepository
    async fn init_surreal_foreign_online_db() -> impl foreign::OnlineRepository {
        let cfg = bcr_wdc_utils::surreal::DBConnConfig {
            connection: "mem://".to_string(),
            namespace: "test".to_string(),
            database: "test".to_string(),
        };
        surreal::DBForeignOnline::new(cfg).await.unwrap()
    }
    fn init_inmemory_foreign_online_db() -> impl foreign::OnlineRepository {
        inmemory::OnlineRepository::default()
    }

    #[tokio::test]
    async fn test_foreign_online_store() {
        let db = init_inmemory_foreign_online_db();
        foreign_online_store(db).await;
        let db = init_surreal_foreign_online_db().await;
        foreign_online_store(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_foreign_online_store_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBForeignOnline::from_pool(pool);
        foreign_online_store(db).await;
    }
    async fn foreign_online_store(db: impl foreign::OnlineRepository) {
        let mint_id = core::generate_random_keypair().public_key();
        let proofs = generate_test_proofs(2);
        db.store(mint_id, proofs).await.unwrap();
        let proofs = generate_test_proofs(2);
        db.store(mint_id, proofs).await.unwrap();
        let pfs = db.list(mint_id).await.unwrap();
        assert_eq!(pfs.len(), 4);
    }

    #[tokio::test]
    async fn test_foreign_online_settled_balance() {
        let db = init_inmemory_foreign_online_db();
        foreign_online_settled_balance(db).await;
        let db = init_surreal_foreign_online_db().await;
        foreign_online_settled_balance(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_foreign_online_settled_balance_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBForeignOnline::from_pool(pool);
        foreign_online_settled_balance(db).await;
    }
    async fn foreign_online_settled_balance(db: impl foreign::OnlineRepository) {
        assert!(db.settled_balance().await.unwrap().is_empty());

        let first = core::generate_random_keypair().public_key();
        let second = core::generate_random_keypair().public_key();
        db.store(first, generate_test_proofs(2)).await.unwrap();
        db.store(first, generate_test_proofs(3)).await.unwrap();
        db.store(second, generate_test_proofs(1)).await.unwrap();

        let balances = db.settled_balance().await.unwrap();
        assert_eq!(balances.len(), 2);
        assert_eq!(balances.get(&first), Some(&cashu::Amount::from(40u64)));
        assert_eq!(balances.get(&second), Some(&cashu::Amount::from(8u64)));
    }

    #[tokio::test]
    async fn test_foreign_online_store_search_htlc() {
        let db = init_inmemory_foreign_online_db();
        foreign_online_store_search_htlc(db).await;
        let db = init_surreal_foreign_online_db().await;
        foreign_online_store_search_htlc(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_foreign_online_store_search_htlc_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBForeignOnline::from_pool(pool);
        foreign_online_store_search_htlc(db).await;
    }
    async fn foreign_online_store_search_htlc(db: impl foreign::OnlineRepository) {
        let hash = bitcoin::hashes::sha256::Hash::const_hash(b"online-htlc");
        let first_mint = core::generate_random_keypair().public_key();
        let proofs = generate_test_proofs(2);
        db.store_htlc(first_mint, hash, proofs.clone())
            .await
            .unwrap();
        let stored = db.search_htlc(&hash).await.unwrap();
        assert_eq!(stored.len(), proofs.len());
        for proof in &proofs {
            assert!(stored.contains(&(first_mint, proof.clone())));
        }
    }

    #[tokio::test]
    async fn test_foreign_online_remove_htlcs() {
        let db = init_inmemory_foreign_online_db();
        foreign_online_remove_htlcs(db).await;
        let db = init_surreal_foreign_online_db().await;
        foreign_online_remove_htlcs(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_foreign_online_remove_htlcs_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBForeignOnline::from_pool(pool);
        foreign_online_remove_htlcs(db).await;
    }
    async fn foreign_online_remove_htlcs(db: impl foreign::OnlineRepository) {
        let hash = bitcoin::hashes::sha256::Hash::const_hash(b"remove-online-htlcs");
        let mint_id = core::generate_random_keypair().public_key();
        let proofs = generate_test_proofs(2);
        let y = proofs[0].y().unwrap();
        db.store_htlc(mint_id, hash, proofs.clone()).await.unwrap();
        db.remove_htlcs(&[]).await.unwrap();
        assert_eq!(db.search_htlc(&hash).await.unwrap().len(), 2);
        db.remove_htlcs(&[y]).await.unwrap();
        let remaining = db.search_htlc(&hash).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0], (mint_id, proofs[1].clone()));
    }

    #[tokio::test]
    async fn test_foreign_online_issued_by_hash() {
        let db = init_inmemory_foreign_online_db();
        foreign_online_issued_by_hash(db).await;
        let db = init_surreal_foreign_online_db().await;
        foreign_online_issued_by_hash(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_foreign_online_issued_by_hash_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBForeignOnline::from_pool(pool);
        foreign_online_issued_by_hash(db).await;
    }
    // An unlocked exchange withdraws its own issuance from reclaim, and only its own.
    async fn foreign_online_issued_by_hash(db: impl foreign::OnlineRepository) {
        let unlocked = bitcoin::hashes::sha256::Hash::const_hash(b"unlocked");
        let abandoned = bitcoin::hashes::sha256::Hash::const_hash(b"abandoned");
        let locktime = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
        let unlocked_proofs = generate_test_proofs(2);
        let abandoned_proofs = generate_test_proofs(1);
        db.store_issued(unlocked, locktime, unlocked_proofs)
            .await
            .unwrap();
        db.store_issued(abandoned, locktime, abandoned_proofs.clone())
            .await
            .unwrap();
        let now = time::OffsetDateTime::now_utc();
        assert_eq!(db.list_expired_issued(now).await.unwrap().len(), 3);

        db.remove_issued_by_hash(&unlocked).await.unwrap();
        assert_eq!(db.list_expired_issued(now).await.unwrap(), abandoned_proofs);
    }

    #[tokio::test]
    async fn test_foreign_online_issued_locktime() {
        let db = init_inmemory_foreign_online_db();
        foreign_online_issued_locktime(db).await;
        let db = init_surreal_foreign_online_db().await;
        foreign_online_issued_locktime(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_foreign_online_issued_locktime_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBForeignOnline::from_pool(pool);
        foreign_online_issued_locktime(db).await;
    }
    // eCash the recipient can still unlock is never offered up for reclaim.
    async fn foreign_online_issued_locktime(db: impl foreign::OnlineRepository) {
        let hash = bitcoin::hashes::sha256::Hash::const_hash(b"issued-locktime");
        let now = time::OffsetDateTime::now_utc();
        let proofs = generate_test_proofs(2);
        db.store_issued(hash, now + time::Duration::hours(1), proofs.clone())
            .await
            .unwrap();
        assert!(db.list_expired_issued(now).await.unwrap().is_empty());
        assert_eq!(
            db.list_expired_issued(now + time::Duration::hours(2))
                .await
                .unwrap()
                .len(),
            2
        );
        db.remove_issued(&[proofs[0].y().unwrap()]).await.unwrap();
        assert_eq!(
            db.list_expired_issued(now + time::Duration::hours(2))
                .await
                .unwrap(),
            vec![proofs[1].clone()]
        );
    }

    #[tokio::test]
    async fn test_foreign_online_store_issued_idempotent() {
        let db = init_inmemory_foreign_online_db();
        foreign_online_store_issued_idempotent(db).await;
        let db = init_surreal_foreign_online_db().await;
        foreign_online_store_issued_idempotent(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_foreign_online_store_issued_idempotent_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBForeignOnline::from_pool(pool);
        foreign_online_store_issued_idempotent(db).await;
    }
    async fn foreign_online_store_issued_idempotent(db: impl foreign::OnlineRepository) {
        let hash = bitcoin::hashes::sha256::Hash::const_hash(b"issued-replay");
        let now = time::OffsetDateTime::now_utc();
        let proofs = generate_test_proofs(2);
        for _ in 0..2 {
            db.store_issued(hash, now, proofs.clone()).await.unwrap();
        }
        let expired = db
            .list_expired_issued(now + time::Duration::hours(1))
            .await
            .unwrap();
        assert_eq!(expired.len(), 2);
    }

    //////////////////////////////////////////////////////////////////// foreign::OfflineRepository
    async fn init_surreal_foreign_offline_db() -> impl foreign::OfflineRepository {
        let cfg = bcr_wdc_utils::surreal::DBConnConfig {
            connection: "mem://".to_string(),
            namespace: "test".to_string(),
            database: "test".to_string(),
        };
        surreal::DBForeignOffline::new(cfg).await.unwrap()
    }

    #[tokio::test]
    async fn test_foreign_offline_claim_redemption() {
        foreign_offline_claim_redemption(inmemory::OfflineRepository::default()).await;
        foreign_offline_claim_redemption(init_surreal_foreign_offline_db().await).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_foreign_offline_claim_redemption_sqlx(pool: ::sqlx::PgPool) {
        foreign_offline_claim_redemption(sqlx::DBForeignOffline::from_pool(pool)).await;
    }
    // The second claim on a digest never gets through.
    async fn foreign_offline_claim_redemption(db: impl foreign::OfflineRepository) {
        assert!(db.claim_redemption([7u8; 32]).await.unwrap());
        assert!(!db.claim_redemption([7u8; 32]).await.unwrap());
        assert!(db.claim_redemption([8u8; 32]).await.unwrap());
    }

    #[tokio::test]
    async fn test_foreign_offline_unsettled_balance() {
        foreign_offline_unsettled_balance(inmemory::OfflineRepository::default()).await;
        foreign_offline_unsettled_balance(init_surreal_foreign_offline_db().await).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_foreign_offline_unsettled_balance_sqlx(pool: ::sqlx::PgPool) {
        foreign_offline_unsettled_balance(sqlx::DBForeignOffline::from_pool(pool)).await;
    }
    async fn foreign_offline_unsettled_balance(db: impl foreign::OfflineRepository) {
        assert!(db.unsettled_balance().await.unwrap().is_empty());

        let first = core::generate_random_keypair().public_key();
        let second = core::generate_random_keypair().public_key();
        db.store_proofs(first, generate_test_proofs(2))
            .await
            .unwrap();
        db.store_proofs(first, generate_test_proofs(3))
            .await
            .unwrap();
        db.store_proofs(second, generate_test_proofs(1))
            .await
            .unwrap();
        let balances = db.unsettled_balance().await.unwrap();
        assert_eq!(balances.len(), 2);
        assert_eq!(balances.get(&first), Some(&cashu::Amount::from(40u64)));
        assert_eq!(balances.get(&second), Some(&cashu::Amount::from(8u64)));
    }

    #[tokio::test]
    async fn test_foreign_offline_reserve_exchange() {
        foreign_offline_reserve_exchange(inmemory::OfflineRepository::default()).await;
        foreign_offline_reserve_exchange(init_surreal_foreign_offline_db().await).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_foreign_offline_reserve_exchange_sqlx(pool: ::sqlx::PgPool) {
        foreign_offline_reserve_exchange(sqlx::DBForeignOffline::from_pool(pool)).await;
    }
    async fn foreign_offline_reserve_exchange(db: impl foreign::OfflineRepository) {
        let proofs = generate_test_proofs(3);
        let ys: Vec<cashu::PublicKey> = proofs.iter().map(|p| p.y().unwrap()).collect();
        let fps = |idx: &[usize]| -> Vec<_> {
            idx.iter()
                .map(|i| wire_keys::ProofFingerprint::try_from(proofs[*i].clone()).unwrap())
                .collect()
        };
        let hashes = |idx: &[usize]| -> Vec<_> {
            idx.iter()
                .map(|i| bitcoin::hashes::sha256::Hash::const_hash(proofs[*i].secret.as_bytes()))
                .collect()
        };
        let exchange = |digest: [u8; 32]| foreign::OfflineExchange {
            exchange_digest: digest,
            alpha_id: core::generate_random_keypair().public_key(),
            evidence_digest: [2u8; 32],
            expires_at: time::OffsetDateTime::from_unix_timestamp(1_900_000_000).unwrap(),
            proofs: generate_test_proofs(2),
        };
        let (first, second) = (exchange([1u8; 32]), exchange([3u8; 32]));
        assert_eq!(db.search_exchange(&ys).await.unwrap(), None);
        assert!(db
            .reserve_exchange(first.clone(), fps(&[0]), hashes(&[0]))
            .await
            .unwrap());
        assert!(!db
            .reserve_exchange(second.clone(), fps(&[1, 0]), hashes(&[1, 0]))
            .await
            .unwrap());
        assert_eq!(db.search_exchange(&ys[1..2]).await.unwrap(), None);
        assert!(db.search_fp(&hashes(&[1])[0]).await.unwrap().is_none());
        assert!(db
            .reserve_exchange(second.clone(), fps(&[1, 2]), hashes(&[1, 2]))
            .await
            .unwrap());
        assert_eq!(db.search_exchange(&ys[..1]).await.unwrap(), Some(first));
        assert_eq!(
            db.search_exchange(&ys[2..]).await.unwrap(),
            Some(second.clone())
        );
        let (alpha_id, _) = db.search_fp(&hashes(&[2])[0]).await.unwrap().unwrap();
        assert_eq!(alpha_id, second.alpha_id);
    }

    #[tokio::test]
    async fn test_foreign_offline_store_load_remove_proofs() {
        foreign_offline_store_load_remove_proofs(inmemory::OfflineRepository::default()).await;
        foreign_offline_store_load_remove_proofs(init_surreal_foreign_offline_db().await).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_foreign_offline_store_load_remove_proofs_sqlx(pool: ::sqlx::PgPool) {
        foreign_offline_store_load_remove_proofs(sqlx::DBForeignOffline::from_pool(pool)).await;
    }
    async fn foreign_offline_store_load_remove_proofs(db: impl foreign::OfflineRepository) {
        assert!(db.list_foreign_pks().await.unwrap().is_empty());
        let first = core::generate_random_keypair().public_key();
        let second = core::generate_random_keypair().public_key();
        let first_proofs = generate_test_proofs(3);
        let second_proofs = generate_test_proofs(1);
        db.store_proofs(first, first_proofs.clone()).await.unwrap();
        db.store_proofs(second, second_proofs.clone())
            .await
            .unwrap();

        let mut pks = db.list_foreign_pks().await.unwrap();
        pks.sort();
        let mut expected = vec![first, second];
        expected.sort();
        assert_eq!(pks, expected);
        let loaded = db.load_proofs(first).await.unwrap();
        assert_eq!(loaded.len(), first_proofs.len());
        for proof in &first_proofs {
            assert!(loaded.contains(proof));
        }

        let removed: Vec<cashu::PublicKey> =
            first_proofs[..2].iter().map(|p| p.y().unwrap()).collect();
        db.remove_proofs(&removed).await.unwrap();
        assert_eq!(db.load_proofs(first).await.unwrap(), first_proofs[2..]);
        assert_eq!(db.load_proofs(second).await.unwrap(), second_proofs);
        let balances = db.unsettled_balance().await.unwrap();
        assert_eq!(balances.get(&first), Some(&cashu::Amount::from(8u64)));
    }

    #[tokio::test]
    async fn test_foreign_offline_store_proofs_twice() {
        // the in-memory repository does not check for duplicates
        foreign_offline_store_proofs_twice(init_surreal_foreign_offline_db().await).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_foreign_offline_store_proofs_twice_sqlx(pool: ::sqlx::PgPool) {
        foreign_offline_store_proofs_twice(sqlx::DBForeignOffline::from_pool(pool)).await;
    }
    // A proof already stored makes the whole batch fail.
    async fn foreign_offline_store_proofs_twice(db: impl foreign::OfflineRepository) {
        let mint_id = core::generate_random_keypair().public_key();
        let proofs = generate_test_proofs(2);
        db.store_proofs(mint_id, proofs[..1].to_vec())
            .await
            .unwrap();
        assert!(db.store_proofs(mint_id, proofs.clone()).await.is_err());
        assert_eq!(db.load_proofs(mint_id).await.unwrap(), proofs[..1]);
    }

    #[tokio::test]
    async fn test_foreign_offline_remove_fps() {
        foreign_offline_remove_fps(inmemory::OfflineRepository::default()).await;
        foreign_offline_remove_fps(init_surreal_foreign_offline_db().await).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_foreign_offline_remove_fps_sqlx(pool: ::sqlx::PgPool) {
        foreign_offline_remove_fps(sqlx::DBForeignOffline::from_pool(pool)).await;
    }
    // Redeemed fingerprints go, the exchange that reserved them stays.
    async fn foreign_offline_remove_fps(db: impl foreign::OfflineRepository) {
        let (exchange, fps, hashes) = generate_offline_exchange(2);
        assert!(db
            .reserve_exchange(exchange.clone(), fps.clone(), hashes.clone())
            .await
            .unwrap());
        assert_eq!(
            db.search_fp(&hashes[0]).await.unwrap(),
            Some((exchange.alpha_id, fps[0].clone()))
        );
        db.remove_fps(&[cashu::PublicKey::from(fps[0].y)])
            .await
            .unwrap();
        assert_eq!(db.search_fp(&hashes[0]).await.unwrap(), None);
        assert_eq!(
            db.search_fp(&hashes[1]).await.unwrap(),
            Some((exchange.alpha_id, fps[1].clone()))
        );
        let ys = [cashu::PublicKey::from(fps[0].y)];
        assert_eq!(db.search_exchange(&ys).await.unwrap(), Some(exchange));
    }

    /// An exchange over `n` fresh inputs; the first fingerprint carries a DLEQ proof.
    fn generate_offline_exchange(
        n: usize,
    ) -> (
        foreign::OfflineExchange,
        Vec<wire_keys::ProofFingerprint>,
        Vec<bitcoin::hashes::sha256::Hash>,
    ) {
        let inputs = generate_test_proofs(n);
        let mut fps: Vec<wire_keys::ProofFingerprint> = inputs
            .iter()
            .map(|p| wire_keys::ProofFingerprint::try_from(p.clone()).unwrap())
            .collect();
        fps[0].dleq = Some(cashu::ProofDleq::new(
            cashu::SecretKey::generate(),
            cashu::SecretKey::generate(),
            cashu::SecretKey::generate(),
        ));
        let hashes = inputs
            .iter()
            .map(|p| bitcoin::hashes::sha256::Hash::const_hash(p.secret.as_bytes()))
            .collect();
        let exchange = foreign::OfflineExchange {
            exchange_digest: rand::random(),
            alpha_id: core::generate_random_keypair().public_key(),
            evidence_digest: rand::random(),
            expires_at: time::OffsetDateTime::from_unix_timestamp(1_900_000_000).unwrap(),
            proofs: generate_test_proofs(2),
        };
        (exchange, fps, hashes)
    }

    // Same dump -> store sequence as the foreign offline step of `bin/migrate.rs`.
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_foreign_offline_migration_roundtrip(pool: ::sqlx::PgPool) {
        use foreign::OfflineRepository as _;
        let cfg = bcr_wdc_utils::surreal::DBConnConfig {
            connection: "mem://".to_string(),
            namespace: "test".to_string(),
            database: "test".to_string(),
        };
        let source = surreal::DBForeignOffline::new(cfg).await.unwrap();
        let mint_id = core::generate_random_keypair().public_key();
        let proofs = generate_test_proofs(3);
        source.store_proofs(mint_id, proofs.clone()).await.unwrap();
        assert!(source.claim_redemption([5u8; 32]).await.unwrap());
        let (first, first_fps, first_hashes) = generate_offline_exchange(2);
        let (second, second_fps, second_hashes) = generate_offline_exchange(1);
        assert!(source
            .reserve_exchange(first.clone(), first_fps.clone(), first_hashes.clone())
            .await
            .unwrap());
        assert!(source
            .reserve_exchange(second.clone(), second_fps.clone(), second_hashes.clone())
            .await
            .unwrap());
        // redeemed: the exchange stays, the fingerprint goes
        source
            .remove_fps(&[cashu::PublicKey::from(first_fps[1].y)])
            .await
            .unwrap();

        let dest = sqlx::DBForeignOffline::from_pool(pool);
        for (mint_id, proof) in source.dump_proofs().await.unwrap() {
            dest.store_proofs(mint_id, vec![proof]).await.unwrap();
        }
        for digest in source.dump_redemptions().await.unwrap() {
            assert!(dest.claim_redemption(digest).await.unwrap());
        }
        let exchanges = source.dump_exchanges().await.unwrap();
        assert_eq!(exchanges.len(), 2);
        for (exchange, ys) in exchanges {
            dest.insert_exchange(exchange, ys).await.unwrap();
        }
        let fps = source.dump_fps().await.unwrap();
        assert_eq!(fps.len(), 2);
        for (hash, mint_id, fp) in fps {
            dest.insert_fp(hash, mint_id, fp).await.unwrap();
        }

        let loaded = dest.load_proofs(mint_id).await.unwrap();
        assert_eq!(loaded.len(), proofs.len());
        for proof in &proofs {
            assert!(loaded.contains(proof));
        }
        assert!(!dest.claim_redemption([5u8; 32]).await.unwrap());
        for fp in &first_fps {
            let ys = [cashu::PublicKey::from(fp.y)];
            assert_eq!(
                dest.search_exchange(&ys).await.unwrap(),
                Some(first.clone())
            );
        }
        let ys = [cashu::PublicKey::from(second_fps[0].y)];
        assert_eq!(
            dest.search_exchange(&ys).await.unwrap(),
            Some(second.clone())
        );
        assert_eq!(
            dest.search_fp(&first_hashes[0]).await.unwrap(),
            Some((first.alpha_id, first_fps[0].clone()))
        );
        assert_eq!(dest.search_fp(&first_hashes[1]).await.unwrap(), None);
        assert_eq!(
            dest.search_fp(&second_hashes[0]).await.unwrap(),
            Some((second.alpha_id, second_fps[0].clone()))
        );
        // a migrated reservation still locks its ys
        let (third, _, _) = generate_offline_exchange(1);
        assert!(!dest
            .reserve_exchange(third, first_fps[..1].to_vec(), first_hashes[..1].to_vec())
            .await
            .unwrap());
    }

    //////////////////////////////////////////////////////////////////// ebill::Repository
    async fn init_surreal_ebill_db() -> impl ebill::Repository {
        let sdb = surrealdb::Surreal::<surrealdb::engine::any::Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        surreal::DBEbill { db: sdb }
    }
    fn init_inmemory_ebill_db() -> impl ebill::Repository {
        inmemory::EbillMintOpMap::default()
    }

    #[tokio::test]
    async fn test_ebill_mint_store() {
        let db = init_inmemory_ebill_db();
        ebill_mint_store(db).await;
        let db = init_surreal_ebill_db().await;
        ebill_mint_store(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_ebill_mint_store_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBEbill::from_pool(pool);
        ebill_mint_store(db).await;
    }
    async fn ebill_mint_store(db: impl ebill::Repository) {
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core::generate_random_keypair();
        let op = ebill::MintOperation {
            uid: Uuid::new_v4(),
            kid: kid.into(),
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op).await.unwrap();
    }

    #[tokio::test]
    async fn test_ebill_mint_store_twice() {
        let db = init_inmemory_ebill_db();
        ebill_mint_store_twice(db).await;
        let db = init_surreal_ebill_db().await;
        ebill_mint_store_twice(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_ebill_mint_store_twice_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBEbill::from_pool(pool);
        ebill_mint_store_twice(db).await;
    }
    async fn ebill_mint_store_twice(db: impl ebill::Repository) {
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core::generate_random_keypair();
        let op = ebill::MintOperation {
            uid: Uuid::new_v4(),
            kid: kid.into(),
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op.clone()).await.unwrap();
        let res = db.mint_store(op).await;
        assert!(matches!(res, Err(Error::AlreadyExists(_))));
    }

    #[tokio::test]
    async fn test_ebill_mint_store_bill_conflict() {
        let db = init_inmemory_ebill_db();
        ebill_mint_store_bill_conflict(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_ebill_mint_store_bill_conflict_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBEbill::from_pool(pool);
        ebill_mint_store_bill_conflict(db).await;
    }
    async fn ebill_mint_store_bill_conflict(db: impl ebill::Repository) {
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core::generate_random_keypair();
        let op = ebill::MintOperation {
            uid: Uuid::new_v4(),
            kid: kid.into(),
            pub_key: kp.public_key().into(),
            target: cashu::Amount::from(64),
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op.clone()).await.unwrap();
        let other = ebill::MintOperation {
            uid: Uuid::new_v4(),
            ..op
        };
        let res = db.mint_store(other).await;
        assert!(matches!(res, Err(Error::AlreadyExists(_))));
    }

    #[tokio::test]
    async fn test_ebill_mint_lookup_by_bill() {
        let db = init_inmemory_ebill_db();
        ebill_mint_lookup_by_bill(db).await;
        let db = init_surreal_ebill_db().await;
        ebill_mint_lookup_by_bill(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_ebill_mint_lookup_by_bill_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBEbill::from_pool(pool);
        ebill_mint_lookup_by_bill(db).await;
    }
    async fn ebill_mint_lookup_by_bill(db: impl ebill::Repository) {
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core::generate_random_keypair();
        let op = ebill::MintOperation {
            uid: Uuid::new_v4(),
            kid: kid.into(),
            pub_key: kp.public_key().into(),
            target: cashu::Amount::from(64),
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        let absent = db.mint_lookup_by_bill(op.bill_id.clone()).await.unwrap();
        assert!(absent.is_none());
        db.mint_store(op.clone()).await.unwrap();
        let found = db.mint_lookup_by_bill(op.bill_id.clone()).await.unwrap();
        assert_eq!(found, Some(op));
        let other = db
            .mint_lookup_by_bill(bcr_common::core_tests::random_bill_id())
            .await
            .unwrap();
        assert!(other.is_none());
    }

    #[tokio::test]
    async fn test_ebill_mint_load() {
        let db = init_inmemory_ebill_db();
        ebill_mint_load(db).await;
        let db = init_surreal_ebill_db().await;
        ebill_mint_load(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_ebill_mint_load_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBEbill::from_pool(pool);
        ebill_mint_load(db).await;
    }
    async fn ebill_mint_load(db: impl ebill::Repository) {
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core::generate_random_keypair();
        let op = ebill::MintOperation {
            uid: Uuid::new_v4(),
            kid: kid.into(),
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op.clone()).await.unwrap();
        let res = db.mint_load(op.uid).await.unwrap();
        assert_eq!(res.kid, kid.into());
        assert_eq!(res.pub_key, kp.public_key().into());
    }

    #[tokio::test]
    async fn test_ebill_mint_update_field() {
        let db = init_inmemory_ebill_db();
        ebill_mint_update_field(db).await;
        let db = init_surreal_ebill_db().await;
        ebill_mint_update_field(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_ebill_mint_update_field_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBEbill::from_pool(pool);
        ebill_mint_update_field(db).await;
    }
    async fn ebill_mint_update_field(db: impl ebill::Repository) {
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core::generate_random_keypair();
        let op = ebill::MintOperation {
            uid: Uuid::new_v4(),
            kid: kid.into(),
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op.clone()).await.unwrap();
        db.mint_update_field(op.uid, cashu::Amount::ZERO, cashu::Amount::from(100u64))
            .await
            .unwrap();
        let res = db.mint_load(op.uid).await.unwrap();
        assert_eq!(res.kid, kid.into());
        assert_eq!(res.minted, cashu::Amount::from(100u64));
    }

    #[tokio::test]
    async fn test_ebill_mint_list() {
        let db = init_inmemory_ebill_db();
        ebill_mint_list(db).await;
        let db = init_surreal_ebill_db().await;
        ebill_mint_list(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_ebill_mint_list_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBEbill::from_pool(pool);
        ebill_mint_list(db).await;
    }
    async fn ebill_mint_list(db: impl ebill::Repository) {
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let kp = core::generate_random_keypair();
        let op1 = ebill::MintOperation {
            uid: Uuid::new_v4(),
            kid: kid.into(),
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op1.clone()).await.unwrap();
        let op2 = ebill::MintOperation {
            uid: Uuid::new_v4(),
            kid: kid.into(),
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op2.clone()).await.unwrap();
        let res = db.mint_list(kid.into()).await.unwrap();
        assert_eq!(res.len(), 2);
        let rids: Vec<_> = res.iter().map(|op| op.uid).collect();
        assert!(rids.contains(&op1.uid));
        assert!(rids.contains(&op2.uid));
    }

    //////////////////////////////////////////////////////////////////// vault::Repository
    async fn init_surreal_vault_db() -> impl vault::Repository {
        let sdb = surrealdb::Surreal::<surrealdb::engine::any::Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        surreal::DBVault { db: sdb }
    }
    fn init_inmemory_vault_db() -> impl vault::Repository {
        inmemory::VaultMap::default()
    }

    fn generate_test_proofs(n: usize) -> Vec<cashu::Proof> {
        let (_, keyset) = core_tests::generate_random_ecash_keyset();
        let amounts = vec![cashu::Amount::from(8u64); n];
        core_tests::generate_random_ecash_proofs(&keyset, &amounts)
    }

    #[tokio::test]
    async fn test_vault_store_load_proofs() {
        let db = init_inmemory_vault_db();
        vault_store_load_proofs(db).await;
        let db = init_surreal_vault_db().await;
        vault_store_load_proofs(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_vault_store_load_proofs_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBVault::from_pool(pool);
        vault_store_load_proofs(db).await;
    }
    async fn vault_store_load_proofs(db: impl vault::Repository) {
        let proofs = generate_test_proofs(3);
        let ys: Vec<cashu::PublicKey> = proofs.iter().map(|p| p.y().unwrap()).collect();
        db.store_proofs(proofs.clone()).await.unwrap();
        let loaded = db.load_proofs(vec![]).await.unwrap();
        assert!(loaded.is_empty());
        let loaded = db.load_proofs(ys).await.unwrap();
        assert_eq!(loaded.len(), 3);
        for proof in &proofs {
            assert!(loaded.contains(proof));
        }
    }

    #[tokio::test]
    async fn test_vault_load_proofs_partial() {
        let db = init_inmemory_vault_db();
        vault_load_proofs_partial(db).await;
        let db = init_surreal_vault_db().await;
        vault_load_proofs_partial(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_vault_load_proofs_partial_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBVault::from_pool(pool);
        vault_load_proofs_partial(db).await;
    }
    async fn vault_load_proofs_partial(db: impl vault::Repository) {
        let proofs = generate_test_proofs(3);
        let ys: Vec<cashu::PublicKey> = proofs.iter().map(|p| p.y().unwrap()).collect();
        db.store_proofs(proofs.clone()).await.unwrap();
        let mut all_ys = ys.clone();
        let extra_y = cashu::PublicKey::from(core::generate_random_keypair().public_key());
        all_ys.push(extra_y);
        let loaded = db.load_proofs(all_ys).await.unwrap();
        assert_eq!(loaded.len(), 3);
    }

    #[tokio::test]
    async fn test_vault_list_ys() {
        let db = init_inmemory_vault_db();
        vault_list_ys(db).await;
        let db = init_surreal_vault_db().await;
        vault_list_ys(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_vault_list_ys_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBVault::from_pool(pool);
        vault_list_ys(db).await;
    }
    async fn vault_list_ys(db: impl vault::Repository) {
        let ys = db.list_ys().await.unwrap();
        assert!(ys.is_empty());
        let proofs = generate_test_proofs(2);
        db.store_proofs(proofs.clone()).await.unwrap();
        let ys = db.list_ys().await.unwrap();
        assert_eq!(ys.len(), 2);
        for proof in &proofs {
            assert!(ys.contains(&proof.y().unwrap()));
        }
    }

    #[tokio::test]
    async fn test_vault_delete_proofs() {
        let db = init_inmemory_vault_db();
        vault_delete_proofs(db).await;
        let db = init_surreal_vault_db().await;
        vault_delete_proofs(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_vault_delete_proofs_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBVault::from_pool(pool);
        vault_delete_proofs(db).await;
    }
    async fn vault_delete_proofs(db: impl vault::Repository) {
        let proofs = generate_test_proofs(3);
        let ys: Vec<cashu::PublicKey> = proofs.iter().map(|p| p.y().unwrap()).collect();
        db.store_proofs(proofs.clone()).await.unwrap();
        let to_delete = &ys[..2];
        db.delete_proofs(to_delete).await.unwrap();
        let remaining_ys = db.list_ys().await.unwrap();
        assert_eq!(remaining_ys.len(), 1);
        assert!(remaining_ys.contains(&ys[2]));
    }

    //////////////////////////////////////////////////////////////////// onchain::Repository
    async fn init_surreal_onchain_db() -> impl onchain::Repository {
        let cfg = bcr_wdc_utils::surreal::DBConnConfig {
            connection: "mem://".to_string(),
            namespace: "test".to_string(),
            database: "test".to_string(),
        };
        surreal::DBOnChain::new(cfg).await.unwrap()
    }

    #[tokio::test]
    async fn test_onchain_update_mintop_status() {
        let db = init_surreal_onchain_db().await;
        onchain_update_mintop_status(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_onchain_update_mintop_status_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBOnChain::from_pool(pool);
        onchain_update_mintop_status(db).await;
    }
    async fn onchain_update_mintop_status(db: impl onchain::Repository) {
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let amounts = vec![cashu::Amount::from(100u64)];
        let blinds = signature_tests::generate_blinds(kid.into(), &amounts)
            .into_iter()
            .map(|(blind, _, _)| blind)
            .map(cashu::BlindedMessage::from)
            .collect();
        let op = onchain::MintOperation {
            qid: Uuid::new_v4(),
            kid: kid.into(),
            target: bitcoin::Amount::ZERO,
            recipient: bitcoin::Address::from_str("n28b7b8HZcrBqeabbjwGRbo8q9JLcusYFC").unwrap(),
            expiry: time::OffsetDateTime::now_utc() + time::Duration::hours(1),
            status: onchain::MintStatus::Pending { blinds },
        };
        db.store_mintop(op.clone()).await.unwrap();
        let mut signatures = core_tests::generate_ecash_signatures(&keys.1, &amounts);
        signatures[0].dleq = Some(cashu::BlindSignatureDleq {
            e: cashu::SecretKey::generate(),
            s: cashu::SecretKey::generate(),
        });
        let status = onchain::MintStatus::Paid {
            signatures: signatures.clone(),
        };
        db.update_mintop_status(op.qid, status).await.unwrap();
        let res = db.load_mintop(op.qid).await.unwrap();
        let onchain::MintStatus::Paid { signatures: loaded } = res.status else {
            panic!("expected Paid status");
        };
        assert_eq!(loaded, signatures);
    }

    #[tokio::test]
    async fn test_onchain_list_pending_mintops() {
        let db = init_surreal_onchain_db().await;
        onchain_list_pending_mintops(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_onchain_list_pending_mintops_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBOnChain::from_pool(pool);
        onchain_list_pending_mintops(db).await;
    }
    async fn onchain_list_pending_mintops(db: impl onchain::Repository) {
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let amounts = vec![cashu::Amount::from(100u64)];
        let now = time::OffsetDateTime::now_utc();
        let blinds = signature_tests::generate_blinds(kid.into(), &amounts)
            .into_iter()
            .map(|(blind, _, _)| blind)
            .map(cashu::BlindedMessage::from)
            .collect();
        let op = onchain::MintOperation {
            qid: Uuid::new_v4(),
            kid: kid.into(),
            target: bitcoin::Amount::ZERO,
            recipient: bitcoin::Address::from_str("n28b7b8HZcrBqeabbjwGRbo8q9JLcusYFC").unwrap(),
            expiry: now + time::Duration::hours(1),
            status: onchain::MintStatus::Pending { blinds },
        };
        db.store_mintop(op.clone()).await.unwrap();
        let pending = db.list_pending_mintops(now).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0], op.qid);
    }

    #[tokio::test]
    async fn test_onchain_list_pending_meltops_roundtrip() {
        let db = init_surreal_onchain_db().await;
        onchain_list_pending_meltops_roundtrip(db).await;
    }
    #[::sqlx::test(migrations = "../../migrations")]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_onchain_list_pending_meltops_roundtrip_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBOnChain::from_pool(pool);
        onchain_list_pending_meltops_roundtrip(db).await;
    }
    async fn onchain_list_pending_meltops_roundtrip(db: impl onchain::Repository) {
        let qid = Uuid::new_v4();
        let input_ys = vec![
            cashu::PublicKey::from(core::generate_random_keypair().public_key()),
            cashu::PublicKey::from(core::generate_random_keypair().public_key()),
        ];
        let wallet_key = cashu::PublicKey::from(core::generate_random_keypair().public_key());
        let commitment = signature_tests::random_schnorr_signature();
        let now = time::OffsetDateTime::now_utc();
        let meltop = onchain::MeltOperation {
            qid,
            target: bitcoin::Amount::from_sat(1000),
            available: cashu::Amount::from(2000u64),
            fees: cashu::Amount::from(10u64),
            address: String::from("n28b7b8HZcrBqeabbjwGRbo8q9JLcusYFC"),
            wallet_key,
            commitment,
            expiry: now + time::Duration::hours(1),
            fp_digest: [7u8; 32],
            input_ys,
            status: onchain::MeltStatus::Pending,
        };
        db.store_meltop(meltop, now).await.unwrap();
        let pending = db.list_pending_meltops(now).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0], qid);
        let loaded = db.load_meltop(qid).await.unwrap();
        assert_eq!(loaded.fp_digest, [7u8; 32]);
    }
}
