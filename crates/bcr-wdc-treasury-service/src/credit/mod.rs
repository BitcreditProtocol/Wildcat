// ----- standard library imports
// ----- extra library imports
use async_trait::async_trait;
use bitcoin::bip32 as btc32;
use cashu::{nut00 as cdk00, nut02 as cdk02, Amount};
use uuid::Uuid;
// ----- local modules
mod keys;
pub use keys::{KeySrvc, KeySrvcConfig};
// ----- local imports
use crate::error::{Error, Result};

// ----- end imports

pub type PremintSignatures = (Uuid, Vec<cdk00::BlindSignature>);

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait Repository {
    async fn next_counter(&self, kid: cdk02::Id) -> Result<u32>;
    async fn increment_counter(&self, kid: cdk02::Id, inc: u32) -> Result<()>;

    async fn store_secrets(&self, request_id: Uuid, premint: cdk00::PreMintSecrets) -> Result<()>;
    async fn load_secrets(&self, request_id: Uuid) -> Result<cdk00::PreMintSecrets>;
    async fn delete_secrets(&self, request_id: Uuid) -> Result<()>;

    async fn store_premint_signatures(&self, premint_signature: PremintSignatures) -> Result<()>;
    async fn list_premint_signatures(&self) -> Result<Vec<(Uuid, Vec<cdk00::BlindSignature>)>>;
    async fn delete_premint_signatures(&self, request_id: Uuid) -> Result<()>;

    async fn store_proofs(&self, proofs: Vec<cdk00::Proof>) -> Result<()>;
    async fn list_balance_by_keyset_id(&self) -> Result<Vec<(cdk02::Id, Amount)>>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait KeyService {
    async fn info(&self, kid: cdk02::Id) -> Result<cdk02::KeySetInfo>;
    async fn keys(&self, kid: cdk02::Id) -> Result<cdk02::KeySet>;
}

#[derive(Clone)]
pub struct Service<Repo, KeySrvc> {
    pub xpriv: btc32::Xpriv,
    pub repo: Repo,
    pub keys: KeySrvc,
}

impl<Repo, KeySrvc> Service<Repo, KeySrvc>
where
    Repo: Repository,
{
    pub async fn generate_blinds(
        &self,
        kid: cdk02::Id,
        total: Amount,
    ) -> Result<(Uuid, Vec<cdk00::BlindedMessage>)> {
        let counter = self.repo.next_counter(kid).await?;
        let premint = cdk00::PreMintSecrets::from_xpriv(
            kid,
            counter,
            self.xpriv,
            total,
            &cashu::amount::SplitTarget::default(),
        )
        .map_err(Error::CDK13)?;
        let request_id = Uuid::new_v4();
        let blinds = premint.blinded_messages();
        self.repo.store_secrets(request_id, premint).await?;
        self.repo
            .increment_counter(kid, blinds.len() as u32)
            .await?;
        Ok((request_id, blinds))
    }

    pub async fn store_signatures(
        &self,
        rid: Uuid,
        signatures: Vec<cdk00::BlindSignature>,
        _expiration: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        self.repo
            .store_premint_signatures((rid, signatures))
            .await?;
        Ok(())
    }
}

impl<Repo, KeySrvc> Service<Repo, KeySrvc>
where
    Repo: Repository,
    KeySrvc: KeyService,
{
    pub async fn balance(&self) -> Result<Amount> {
        let premint_signatures = self.repo.list_premint_signatures().await?;
        for (rid, signatures) in &premint_signatures {
            let kid = signatures.first().expect("empty signatures").keyset_id;
            let Ok(result) = self.keys.info(kid).await else {
                continue;
            };
            if !result.active {
                self.repo.delete_premint_signatures(*rid).await?;
                self.repo.delete_secrets(*rid).await?;
                continue;
            }
            let secrets = self.repo.load_secrets(*rid).await?;
            let keys = self.keys.keys(kid).await?;
            let proofs = unblind_signatures(signatures, &secrets, &keys)?;
            self.repo.store_proofs(proofs).await?;
            self.repo.delete_premint_signatures(*rid).await?;
            self.repo.delete_secrets(*rid).await?;
        }
        let mut total = Amount::ZERO;
        let balances = self.repo.list_balance_by_keyset_id().await?;
        for (kid, balance) in &balances {
            let Ok(result) = self.keys.info(*kid).await else {
                continue;
            };
            if !result.active {
                continue;
            }
            total += *balance;
        }
        Ok(total)
    }
}

fn unblind_signatures(
    signatures: &[cdk00::BlindSignature],
    secrets: &cdk00::PreMintSecrets,
    keys: &cdk02::KeySet,
) -> Result<Vec<cdk00::Proof>> {
    if signatures.len() != secrets.secrets.len() {
        return Err(Error::UnblindSignatures(format!(
            "#signatures {} != #secrets {}",
            signatures.len(),
            secrets.secrets.len()
        )));
    }
    let mut proofs = Vec::with_capacity(signatures.len());
    for (signature, secret) in signatures.iter().zip(secrets.secrets.iter()) {
        if signature.keyset_id != secrets.keyset_id {
            return Err(Error::UnblindSignatures(format!(
                "signature.keyset_id {} != secrets.keyset_id {}",
                signature.keyset_id, secrets.keyset_id
            )));
        }
        let key = keys.keys.amount_key(signature.amount).ok_or_else(|| {
            Error::UnblindSignatures(String::from("signature.amount not in keyset"))
        })?;
        let c = cashu::dhke::unblind_message(&signature.c, &secret.r, &key)
            .map_err(|e| Error::UnblindSignatures(format!("unblind_message error: {}", e)))?;
        let proof = cdk00::Proof::new(signature.amount, keys.id, secret.secret.clone(), c);
        proofs.push(proof);
    }
    Ok(proofs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcr_wdc_swap_service::utils as swap_utils;
    use bcr_wdc_utils::keys::test_utils as key_utils;
    use bitcoin::network::Network;
    use itertools::Itertools;

    #[tokio::test]
    async fn balance_emptysignatures() {
        let mut repo = MockRepository::new();
        let keys = MockKeyService::new();
        let xpriv = btc32::Xpriv::new_master(Network::Testnet, &[0; 32]).unwrap();

        repo.expect_list_premint_signatures()
            .returning(|| Ok(vec![]));
        repo.expect_list_balance_by_keyset_id()
            .returning(|| Ok(vec![]));

        let srvc = Service { keys, xpriv, repo };
        let balance = srvc.balance().await.unwrap();
        assert_eq!(balance, Amount::ZERO);
    }

    #[tokio::test]
    async fn balance_signatures_deactive_keyset() {
        let mut repo = MockRepository::new();
        let mut keys = MockKeyService::new();
        let xpriv = btc32::Xpriv::new_master(Network::Testnet, &[0; 32]).unwrap();

        let (info, keyset) = key_utils::generate_random_keyset();
        let signatures = swap_utils::generate_signatures(&keyset, &[Amount::from(16)]);
        let req_id = Uuid::new_v4();
        let premints = vec![(req_id, signatures.clone())];
        repo.expect_list_premint_signatures()
            .returning(move || Ok(premints.clone()));
        let mut kinfo = cdk02::KeySetInfo::from(info);
        kinfo.active = false;
        keys.expect_info().returning(move |_| Ok(kinfo.clone()));
        repo.expect_delete_premint_signatures()
            .withf(move |id| *id == req_id)
            .returning(|_| Ok(()));
        repo.expect_delete_secrets()
            .withf(move |id| *id == req_id)
            .returning(|_| Ok(()));

        repo.expect_list_balance_by_keyset_id()
            .returning(move || Ok(vec![]));

        let srvc = Service { keys, xpriv, repo };
        let balance = srvc.balance().await.unwrap();
        assert_eq!(balance, Amount::from(0));
    }

    #[tokio::test]
    async fn balance_signatures() {
        let mut repo = MockRepository::new();
        let mut keys = MockKeyService::new();
        let xpriv = btc32::Xpriv::new_master(Network::Testnet, &[0; 32]).unwrap();

        let (info, keyset) = key_utils::generate_random_keyset();

        let (blinds, secrets, _): (Vec<_>, Vec<_>, Vec<_>) =
            swap_utils::generate_blinds(&keyset, &[Amount::from(16)])
                .into_iter()
                .map(|(b, s, k)| {
                    (
                        b.clone(),
                        cdk00::PreMint {
                            amount: b.amount,
                            blinded_message: b,
                            r: k.clone(),
                            secret: s.clone(),
                        },
                        k,
                    )
                })
                .multiunzip();
        let signatures = blinds
            .into_iter()
            .map(|b| bcr_wdc_utils::keys::sign_with_keys(&keyset, &b).unwrap())
            .collect();
        let req_id = Uuid::new_v4();
        let premints = vec![(req_id, signatures)];
        repo.expect_list_premint_signatures()
            .returning(move || Ok(premints.clone()));
        let kinfo = cdk02::KeySetInfo::from(info);
        keys.expect_info().returning(move |_| Ok(kinfo.clone()));
        let premint = cdk00::PreMintSecrets {
            secrets,
            keyset_id: keyset.id,
        };
        repo.expect_load_secrets()
            .withf(move |id| *id == req_id)
            .returning(move |_| Ok(premint.clone()));
        let kkeyset = cdk02::KeySet::from(keyset.clone());
        keys.expect_keys()
            .withf(move |id| *id == keyset.id)
            .returning(move |_| Ok(kkeyset.clone()));
        repo.expect_store_proofs().returning(|_| Ok(()));
        repo.expect_delete_premint_signatures()
            .withf(move |id| *id == req_id)
            .returning(|_| Ok(()));
        repo.expect_delete_secrets()
            .withf(move |id| *id == req_id)
            .returning(|_| Ok(()));
        let kid = keyset.id;
        repo.expect_list_balance_by_keyset_id()
            .returning(move || Ok(vec![(kid, Amount::from(16))]));

        let srvc = Service { keys, xpriv, repo };
        let balance = srvc.balance().await.unwrap();
        assert_eq!(balance, Amount::from(16));
    }

    #[tokio::test]
    async fn balance_only_proofs() {
        let mut repo = MockRepository::new();
        let mut keys = MockKeyService::new();
        let xpriv = btc32::Xpriv::new_master(Network::Testnet, &[0; 32]).unwrap();

        repo.expect_list_premint_signatures()
            .returning(|| Ok(vec![]));

        let (info, keyset) = key_utils::generate_random_keyset();
        let kid = keyset.id;
        repo.expect_list_balance_by_keyset_id()
            .returning(move || Ok(vec![(kid, Amount::from(16))]));
        let kinfo = cdk02::KeySetInfo::from(info);
        keys.expect_info()
            .withf(move |k_id| *k_id == kid)
            .returning(move |_| Ok(kinfo.clone()));

        let srvc = Service { keys, xpriv, repo };
        let balance = srvc.balance().await.unwrap();
        assert_eq!(balance, Amount::from(16));
    }
}
