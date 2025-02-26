// ---------- Swap Keys Repository
#[derive(Default, Clone)]
pub struct SwapRepository<KeysRepo, ActiveRepo> {
    pub endorsed_keys: KeysRepo,
    pub maturity_keys: KeysRepo,
    pub debit_keys: ActiveRepo,
}

impl<KeysRepo, ActiveRepo> SwapRepository<KeysRepo, ActiveRepo>
where
    KeysRepo: keys::Repository,
    ActiveRepo: keys::ActiveRepository,
{
    async fn find_maturity_keys_from_maturity_date(
        &self,
        maturity_date: TStamp,
        mut rotation_idx: u32,
    ) -> Result<Option<KeysetID>> {
        let mut kid = keys::generate_keyset_id_from_date(maturity_date, rotation_idx);
        while let Some(info) = self.maturity_keys.info(&kid).await? {
            if info.active {
                return Ok(Some(kid));
            }
            rotation_idx += 1;
            kid = keys::generate_keyset_id_from_date(maturity_date, rotation_idx)
        }
        Ok(None)
    }

    async fn find_maturity_keys_from_id(&self, kid: &KeysetID) -> Result<Option<KeysetID>> {
        if let Some(info) = self.maturity_keys.info(kid).await? {
            if info.active {
                return Ok(Some(*kid));
            }
            let valid_to = info.valid_to.expect("valid_to field not set") as i64;
            let maturity =
                TStamp::from_timestamp(valid_to, 0).expect("datetime conversion from u64");
            let rotation_index = info
                .derivation_path_index
                .expect("derivation_path_index not set");
            return self
                .find_maturity_keys_from_maturity_date(maturity, rotation_index + 1)
                .await;
        }
        Ok(None)
    }
}

#[async_trait]
impl<KeysRepo, ActiveRepo> swap::KeysRepository for SwapRepository<KeysRepo, ActiveRepo>
where
    KeysRepo: keys::Repository,
    ActiveRepo: keys::ActiveRepository,
{
    async fn keyset(&self, id: &KeysetID) -> AnyResult<Option<cdk02::MintKeySet>> {
        if let Some(keyset) = self.endorsed_keys.keyset(id).await? {
            return Ok(Some(keyset));
        }
        if let Some(keyset) = self.maturity_keys.keyset(id).await? {
            return Ok(Some(keyset));
        }
        self.debit_keys.keyset(id).await
    }
    async fn info(&self, id: &KeysetID) -> AnyResult<Option<cdk_mint::MintKeySetInfo>> {
        if let Some(info) = self.endorsed_keys.info(id).await? {
            return Ok(Some(info));
        }
        if let Some(info) = self.maturity_keys.info(id).await? {
            return Ok(Some(info));
        }
        self.debit_keys.info(id).await
    }
    // in case keyset id is inactive, returns the proper replacement for it
    async fn replacing_id(&self, kid: &KeysetID) -> AnyResult<Option<KeysetID>> {
        if let Some(info) = self.endorsed_keys.info(kid).await? {
            let valid_to = info.valid_to.expect("valid_to field not set") as i64;
            let maturity =
                TStamp::from_timestamp(valid_to, 0).expect("datetime conversion from u64");
            if let Some(id) = self
                .find_maturity_keys_from_maturity_date(maturity, 0)
                .await?
            {
                return Ok(Some(id));
            }
        }
        if let Some(kid) = self.find_maturity_keys_from_id(kid).await? {
            return Ok(Some(kid));
        }
        let kid = self
            .debit_keys
            .info_active()
            .await?
            .map(|info| info.id)
            .map(KeysetID::from);
        Ok(kid)
    }
}


    #[tokio::test]
    async fn test_swaprepository_info_debit_key() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let mut debit_repo = keys_test::MockRepository::new();

        let kid = keys_test::generate_random_keysetid();
        let info = cdk_mint::MintKeySetInfo {
            active: true,
            derivation_path: Default::default(),
            derivation_path_index: Default::default(),
            id: kid.into(),
            input_fee_ppk: Default::default(),
            max_order: Default::default(),
            unit: Default::default(),
            valid_from: Default::default(),
            valid_to: Default::default(),
        };

        quote_repo
            .expect_info()
            .with(eq(kid))
            .returning(|_| Ok(None));
        maturing_repo
            .expect_info()
            .with(eq(kid))
            .returning(|_| Ok(None));
        let cinfo = info.clone();
        debit_repo
            .expect_info()
            .with(eq(kid))
            .returning(move |_| Ok(Some(cinfo.clone())));

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.info(&kid).await.unwrap();
        assert_eq!(result, Some(info));
    }

    #[tokio::test]
    async fn test_swaprepository_info_maturing_key() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let debit_repo = keys_test::MockRepository::new();

        let kid = keys_test::generate_random_keysetid();
        let info = cdk_mint::MintKeySetInfo {
            active: true,
            derivation_path: Default::default(),
            derivation_path_index: Default::default(),
            id: kid.into(),
            input_fee_ppk: Default::default(),
            max_order: Default::default(),
            unit: Default::default(),
            valid_from: Default::default(),
            valid_to: Default::default(),
        };

        quote_repo
            .expect_info()
            .with(eq(kid))
            .returning(|_| Ok(None));
        let cinfo = info.clone();
        maturing_repo
            .expect_info()
            .with(eq(kid))
            .returning(move |_| Ok(Some(cinfo.clone())));

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.info(&kid).await.unwrap();
        assert_eq!(result, Some(info));
    }

    #[tokio::test]
    async fn test_swaprepository_info_quote_key() {
        let mut quote_repo = keys_test::MockRepository::new();
        let maturing_repo = keys_test::MockRepository::new();
        let debit_repo = keys_test::MockRepository::new();

        let kid = keys_test::generate_random_keysetid();
        let info = cdk_mint::MintKeySetInfo {
            active: true,
            derivation_path: Default::default(),
            derivation_path_index: Default::default(),
            id: kid.into(),
            input_fee_ppk: Default::default(),
            max_order: Default::default(),
            unit: Default::default(),
            valid_from: Default::default(),
            valid_to: Default::default(),
        };

        let cinfo = info.clone();
        quote_repo
            .expect_info()
            .with(eq(kid))
            .returning(move |_| Ok(Some(cinfo.clone())));

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.info(&kid).await.unwrap();
        assert_eq!(result, Some(info));
    }

    #[tokio::test]
    async fn test_swaprepository_keyset_debit_key() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let mut debit_repo = keys_test::MockRepository::new();

        let kid = keys_test::generate_random_keysetid();
        let set = cdk02::MintKeySet {
            id: kid.into(),
            keys: cdk01::MintKeys::new(Default::default()),
            unit: Default::default(),
        };

        quote_repo
            .expect_keyset()
            .with(eq(kid))
            .returning(|_| Ok(None));
        maturing_repo
            .expect_keyset()
            .with(eq(kid))
            .returning(|_| Ok(None));
        let cset = set.clone();
        debit_repo
            .expect_keyset()
            .with(eq(kid))
            .returning(move |_| Ok(Some(cset.clone())));

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.keyset(&kid).await.unwrap();
        assert_eq!(result, Some(set));
    }

    #[tokio::test]
    async fn test_swaprepository_keyset_maturing_key() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let debit_repo = keys_test::MockRepository::new();

        let kid = keys_test::generate_random_keysetid();
        let set = cdk02::MintKeySet {
            id: kid.into(),
            keys: cdk01::MintKeys::new(Default::default()),
            unit: Default::default(),
        };

        quote_repo
            .expect_keyset()
            .with(eq(kid))
            .returning(|_| Ok(None));
        let cset = set.clone();
        maturing_repo
            .expect_keyset()
            .with(eq(kid))
            .returning(move |_| Ok(Some(cset.clone())));

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.keyset(&kid).await.unwrap();
        assert_eq!(result, Some(set));
    }

    #[tokio::test]
    async fn test_swaprepository_keyset_quote_key() {
        let mut quote_repo = keys_test::MockRepository::new();
        let maturing_repo = keys_test::MockRepository::new();
        let debit_repo = keys_test::MockRepository::new();

        let kid = keys_test::generate_random_keysetid();
        let set = cdk02::MintKeySet {
            id: kid.into(),
            keys: cdk01::MintKeys::new(Default::default()),
            unit: Default::default(),
        };

        let cset = set.clone();
        quote_repo
            .expect_keyset()
            .with(eq(kid))
            .returning(move |_| Ok(Some(cset.clone())));

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.keyset(&kid).await.unwrap();
        assert_eq!(result, Some(set));
    }

    #[tokio::test]
    async fn test_swaprepository_replacing_keys_debit() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let mut debit_repo = keys_test::MockRepository::new();

        let in_kid = keys_test::generate_random_keysetid();
        let out_kid = keys_test::generate_random_keysetid();

        quote_repo
            .expect_info()
            .with(eq(in_kid))
            .returning(|_| Ok(None));
        maturing_repo
            .expect_info()
            .with(eq(in_kid))
            .returning(|_| Ok(None));
        debit_repo.expect_info_active().returning(move || {
            Ok(Some(cdk_mint::MintKeySetInfo {
                active: true,
                derivation_path: Default::default(),
                derivation_path_index: Default::default(),
                id: out_kid.into(),
                input_fee_ppk: Default::default(),
                max_order: Default::default(),
                unit: Default::default(),
                valid_from: Default::default(),
                valid_to: Default::default(),
            }))
        });

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.replacing_id(&in_kid).await.unwrap();
        assert_eq!(result, Some(out_kid));
    }

    #[tokio::test]
    async fn test_swaprepository_replacing_keys_maturing_active() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let debit_repo = keys_test::MockRepository::new();

        let kid = keys_test::generate_random_keysetid();

        quote_repo
            .expect_info()
            .with(eq(kid))
            .returning(|_| Ok(None));
        maturing_repo
            .expect_info()
            .with(eq(kid))
            .returning(move |_| {
                Ok(Some(cdk_mint::MintKeySetInfo {
                    active: true,
                    derivation_path: Default::default(),
                    derivation_path_index: Default::default(),
                    id: kid.into(),
                    input_fee_ppk: Default::default(),
                    max_order: Default::default(),
                    unit: Default::default(),
                    valid_from: Default::default(),
                    valid_to: Default::default(),
                }))
            });

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.replacing_id(&kid).await.unwrap();
        assert_eq!(result, Some(kid));
    }

    #[tokio::test]
    async fn test_swaprepository_replacing_keys_maturing_inactive() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let debit_repo = keys_test::MockRepository::new();

        let in_kid = keys_test::generate_random_keysetid();
        let maturity_date =
            chrono::NaiveDateTime::parse_from_str("2026-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")
                .unwrap()
                .and_utc();

        quote_repo
            .expect_info()
            .with(eq(in_kid))
            .returning(|_| Ok(None));
        maturing_repo
            .expect_info()
            .with(eq(in_kid))
            .returning(move |_| {
                Ok(Some(cdk_mint::MintKeySetInfo {
                    active: false,
                    derivation_path: Default::default(),
                    derivation_path_index: Some(0),
                    id: in_kid.into(),
                    input_fee_ppk: Default::default(),
                    max_order: Default::default(),
                    unit: Default::default(),
                    valid_from: Default::default(),
                    valid_to: Some(maturity_date.timestamp() as u64),
                }))
            });
        let maturity_kid = keys::generate_keyset_id_from_date(maturity_date, 1);
        maturing_repo
            .expect_info()
            .with(eq(maturity_kid))
            .returning(move |_| {
                Ok(Some(cdk_mint::MintKeySetInfo {
                    active: true,
                    derivation_path: Default::default(),
                    derivation_path_index: Some(1),
                    id: maturity_kid.into(),
                    input_fee_ppk: Default::default(),
                    max_order: Default::default(),
                    unit: Default::default(),
                    valid_from: Default::default(),
                    valid_to: Some(maturity_date.timestamp() as u64),
                }))
            });

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.replacing_id(&in_kid).await.unwrap();
        assert_eq!(result, Some(maturity_kid));
    }

    #[tokio::test]
    async fn test_swaprepository_replacing_keys_maturing_inactive_to_debit() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let mut debit_repo = keys_test::MockRepository::new();

        let in_kid = keys_test::generate_random_keysetid();
        let maturity_date =
            chrono::NaiveDateTime::parse_from_str("2026-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")
                .unwrap()
                .and_utc();

        quote_repo
            .expect_info()
            .with(eq(in_kid))
            .returning(|_| Ok(None));
        maturing_repo
            .expect_info()
            .with(eq(in_kid))
            .returning(move |_| {
                Ok(Some(cdk_mint::MintKeySetInfo {
                    active: false,
                    derivation_path: Default::default(),
                    derivation_path_index: Some(0),
                    id: in_kid.into(),
                    input_fee_ppk: Default::default(),
                    max_order: Default::default(),
                    unit: Default::default(),
                    valid_from: Default::default(),
                    valid_to: Some(maturity_date.timestamp() as u64),
                }))
            });
        let maturity_kid = keys::generate_keyset_id_from_date(maturity_date, 1);
        maturing_repo
            .expect_info()
            .with(eq(maturity_kid))
            .returning(move |_| Ok(None));
        let debit_kid = keys_test::generate_random_keysetid();
        debit_repo.expect_info_active().returning(move || {
            Ok(Some(cdk_mint::MintKeySetInfo {
                active: false,
                derivation_path: Default::default(),
                derivation_path_index: Some(0),
                id: debit_kid.into(),
                input_fee_ppk: Default::default(),
                max_order: Default::default(),
                unit: Default::default(),
                valid_from: Default::default(),
                valid_to: Some(maturity_date.timestamp() as u64),
            }))
        });

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.replacing_id(&in_kid).await.unwrap();
        assert_eq!(result, Some(debit_kid));
    }

    #[tokio::test]
    async fn test_swaprepository_replacing_keys_quote_to_maturing() {
        let mut quote_repo = keys_test::MockRepository::new();
        let mut maturing_repo = keys_test::MockRepository::new();
        let debit_repo = keys_test::MockRepository::new();

        let in_kid = keys_test::generate_random_keysetid();
        let maturity_date =
            chrono::NaiveDateTime::parse_from_str("2026-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")
                .unwrap()
                .and_utc();

        quote_repo
            .expect_info()
            .with(eq(in_kid))
            .returning(move |_| {
                Ok(Some(cdk_mint::MintKeySetInfo {
                    active: false,
                    derivation_path: Default::default(),
                    derivation_path_index: Some(0),
                    id: in_kid.into(),
                    input_fee_ppk: Default::default(),
                    max_order: Default::default(),
                    unit: Default::default(),
                    valid_from: Default::default(),
                    valid_to: Some(maturity_date.timestamp() as u64),
                }))
            });
        let maturity_kid = keys::generate_keyset_id_from_date(maturity_date, 0);
        maturing_repo
            .expect_info()
            .with(eq(maturity_kid))
            .returning(move |_| {
                Ok(Some(cdk_mint::MintKeySetInfo {
                    active: true,
                    derivation_path: Default::default(),
                    derivation_path_index: Some(0),
                    id: maturity_kid.into(),
                    input_fee_ppk: Default::default(),
                    max_order: Default::default(),
                    unit: Default::default(),
                    valid_from: Default::default(),
                    valid_to: None,
                }))
            });

        let swap_repo = SwapRepository {
            endorsed_keys: quote_repo,
            maturity_keys: maturing_repo,
            debit_keys: debit_repo,
        };

        let result = swap_repo.replacing_id(&in_kid).await.unwrap();
        assert_eq!(result, Some(maturity_kid));
    }

