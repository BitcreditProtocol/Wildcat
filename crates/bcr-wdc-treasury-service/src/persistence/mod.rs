// ----- standard library imports
// ----- extra library imports
// ----- local imports
// ----- local modules
pub mod inmemory;
pub mod surreal;

// ----- end imports

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ebill::{self, Repository as EbillRepo};
    use crate::error::Error;
    use bcr_common::{cashu, core, core_tests};
    use uuid::Uuid;

    //////////////////////////////////////////////////////////////////// ebill::Repository
    async fn init_surreal_ebill_db() -> impl EbillRepo {
        let sdb = surrealdb::Surreal::<surrealdb::engine::any::Any>::init();
        sdb.connect("mem://").await.unwrap();
        sdb.use_ns("test").await.unwrap();
        sdb.use_db("test").await.unwrap();
        surreal::DBEbill { db: sdb }
    }
    fn init_inmemory_ebill_db() -> impl EbillRepo {
        inmemory::EbillMintOpMap::default()
    }

    #[tokio::test]
    async fn test_ebill_mint_store() {
        let db = init_inmemory_ebill_db();
        ebill_mint_store(db).await;
        let db = init_surreal_ebill_db().await;
        ebill_mint_store(db).await;
    }
    async fn ebill_mint_store(db: impl EbillRepo) {
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
    async fn ebill_mint_store_twice(db: impl EbillRepo) {
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
        assert!(matches!(res, Err(Error::InvalidInput(_))));
    }

    #[tokio::test]
    async fn test_ebill_mint_load() {
        let db = init_inmemory_ebill_db();
        ebill_mint_load(db).await;
        let db = init_surreal_ebill_db().await;
        ebill_mint_load(db).await;
    }
    async fn ebill_mint_load(db: impl EbillRepo) {
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
    async fn ebill_mint_update_field(db: impl EbillRepo) {
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
    async fn ebill_mint_list(db: impl EbillRepo) {
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

    //////////////////////////////////////////////////////////////////// sqlx
}
