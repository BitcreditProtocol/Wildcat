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
    use bcr_common::{cashu, core, core_tests};
    use bcr_wdc_utils::signatures::test_utils as signature_tests;
    use bitcoin::hashes::Hash;
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
    #[::sqlx::test]
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
    async fn test_foreign_online_store_search_htlc() {
        let db = init_inmemory_foreign_online_db();
        foreign_online_store_search_htlc(db).await;
        let db = init_surreal_foreign_online_db().await;
        foreign_online_store_search_htlc(db).await;
    }
    #[::sqlx::test]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_foreign_online_store_search_htlc_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBForeignOnline::from_pool(pool);
        foreign_online_store_search_htlc(db).await;
    }
    async fn foreign_online_store_search_htlc(db: impl foreign::OnlineRepository) {
        let hash = bitcoin::hashes::sha256::Hash::const_hash(b"online-htlc");
        let first_mint = core::generate_random_keypair().public_key();
        let _second_mint = core::generate_random_keypair().public_key();
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
    #[::sqlx::test]
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
    #[::sqlx::test]
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
            kid,
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
    #[::sqlx::test]
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
            kid,
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
    async fn test_ebill_mint_lookup_by_bill() {
        let db = init_inmemory_ebill_db();
        ebill_mint_lookup_by_bill(db).await;
        let db = init_surreal_ebill_db().await;
        ebill_mint_lookup_by_bill(db).await;
    }
    #[::sqlx::test]
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
            kid,
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
    #[::sqlx::test]
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
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op.clone()).await.unwrap();
        let res = db.mint_load(op.uid).await.unwrap();
        assert_eq!(res.kid, kid);
        assert_eq!(res.pub_key, kp.public_key().into());
    }

    #[tokio::test]
    async fn test_ebill_mint_update_field() {
        let db = init_inmemory_ebill_db();
        ebill_mint_update_field(db).await;
        let db = init_surreal_ebill_db().await;
        ebill_mint_update_field(db).await;
    }
    #[::sqlx::test]
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
            kid,
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
        assert_eq!(res.kid, kid);
        assert_eq!(res.minted, cashu::Amount::from(100u64));
    }

    #[tokio::test]
    async fn test_ebill_mint_list() {
        let db = init_inmemory_ebill_db();
        ebill_mint_list(db).await;
        let db = init_surreal_ebill_db().await;
        ebill_mint_list(db).await;
    }
    #[::sqlx::test]
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
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op1.clone()).await.unwrap();
        let op2 = ebill::MintOperation {
            uid: Uuid::new_v4(),
            kid,
            pub_key: kp.public_key().into(),
            target: cashu::Amount::ZERO,
            minted: cashu::Amount::ZERO,
            bill_id: bcr_common::core_tests::random_bill_id(),
        };
        db.mint_store(op2.clone()).await.unwrap();
        let res = db.mint_list(kid).await.unwrap();
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
    #[::sqlx::test]
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
    #[::sqlx::test]
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
    #[::sqlx::test]
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
    #[::sqlx::test]
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
    #[::sqlx::test]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_onchain_update_mintop_status_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBOnChain::from_pool(pool);
        onchain_update_mintop_status(db).await;
    }
    async fn onchain_update_mintop_status(db: impl onchain::Repository) {
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let amounts = vec![cashu::Amount::from(100u64)];
        let blinds = signature_tests::generate_blinds(kid, &amounts)
            .into_iter()
            .map(|(blind, _, _)| blind)
            .collect();
        let op = onchain::MintOperation {
            qid: Uuid::new_v4(),
            kid,
            target: bitcoin::Amount::ZERO,
            recipient: bitcoin::Address::from_str("n28b7b8HZcrBqeabbjwGRbo8q9JLcusYFC").unwrap(),
            expiry: chrono::Utc::now() + chrono::Duration::hours(1),
            status: onchain::MintStatus::Pending { blinds },
        };
        db.store_mintop(op.clone()).await.unwrap();
        let signatures = core_tests::generate_ecash_signatures(&keys.1, &amounts);
        let status = onchain::MintStatus::Paid { signatures };
        db.update_mintop_status(op.qid, status).await.unwrap();
        let res = db.load_mintop(op.qid).await.unwrap();
        assert!(matches!(res.status, onchain::MintStatus::Paid { .. }));
    }

    #[tokio::test]
    async fn test_onchain_list_pending_mintops() {
        let db = init_surreal_onchain_db().await;
        onchain_list_pending_mintops(db).await;
    }
    #[::sqlx::test]
    #[ignore = "requires DATABASE_URL with CREATEDB permission"]
    async fn test_onchain_list_pending_mintops_sqlx(pool: ::sqlx::PgPool) {
        let db = sqlx::DBOnChain::from_pool(pool);
        onchain_list_pending_mintops(db).await;
    }
    async fn onchain_list_pending_mintops(db: impl onchain::Repository) {
        let keys = core_tests::generate_random_ecash_keyset();
        let kid = keys.0.id;
        let amounts = vec![cashu::Amount::from(100u64)];
        let now = chrono::Utc::now();
        let blinds = signature_tests::generate_blinds(kid, &amounts)
            .into_iter()
            .map(|(blind, _, _)| blind)
            .collect();
        let op = onchain::MintOperation {
            qid: Uuid::new_v4(),
            kid,
            target: bitcoin::Amount::ZERO,
            recipient: bitcoin::Address::from_str("n28b7b8HZcrBqeabbjwGRbo8q9JLcusYFC").unwrap(),
            expiry: now + chrono::Duration::hours(1),
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
    #[::sqlx::test]
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
        let now = chrono::Utc::now();
        let meltop = onchain::MeltOperation {
            qid,
            target: bitcoin::Amount::from_sat(1000),
            available: cashu::Amount::from(2000u64),
            fees: cashu::Amount::from(10u64),
            address: String::from("n28b7b8HZcrBqeabbjwGRbo8q9JLcusYFC"),
            wallet_key,
            commitment,
            expiry: now + chrono::Duration::hours(1),
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
