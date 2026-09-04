// ----- standard library imports
// ----- extra library imports
use bcr_common::cashu::{self, ProofsMethods};
use bcr_common::core::NodeId;
// ----- local imports
use crate::{
    error::Result,
    vault::{Repository, WildcatClient},
    TStamp,
};

// ----- end imports

pub struct Service {
    pub repo: Box<dyn Repository>,
    pub wdc_cl: Box<dyn WildcatClient>,
    pub my_url: cashu::MintUrl,
    pub mint_id: NodeId,
}

impl Service {
    async fn clean_local(&self) -> Result<Vec<cashu::PublicKey>> {
        let ys = self.repo.list_ys().await?;
        let states = self.wdc_cl.check_spent(ys).await?;
        let (spent, unspent): (Vec<_>, Vec<_>) = states
            .into_iter()
            .partition(|s| matches!(s.state, cashu::State::Spent));
        let spent_ys: Vec<_> = spent.into_iter().map(|s| s.y).collect();
        self.repo.delete_proofs(&spent_ys).await?;
        let unspent_ys: Vec<_> = unspent.into_iter().map(|s| s.y).collect();
        Ok(unspent_ys)
    }

    pub async fn store_proofs(&self, proofs: Vec<cashu::Proof>) -> Result<()> {
        let ys = proofs.ys()?;
        let states = self.wdc_cl.check_spent(ys).await?;
        let filtered: Vec<cashu::Proof> = proofs
            .into_iter()
            .zip(states)
            .filter_map(|(p, s)| {
                if matches!(s.state, cashu::State::Spent) {
                    None
                } else {
                    Some(p)
                }
            })
            .collect();
        self.repo.store_proofs(filtered).await?;
        Ok(())
    }

    pub async fn generate_token(&self, now: TStamp) -> Result<bcr_common::wallet::Token> {
        let unspent_ys = self.clean_local().await?;
        let proofs = self.repo.load_proofs(unspent_ys).await?;
        let proofs: bcr_common::ecash::Proofs = proofs.into_iter().map(Into::into).collect();
        let memo = format!("Treasury token generated at {now}");
        let token =
            bcr_common::wallet::BitcrTokenV5::new(self.mint_id.clone(), self.wdc_cl.unit(), proofs)
                .with_mint_url(self.my_url.to_string())
                .with_memo(memo);
        Ok(token.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::{MockRepository, MockWildcatClient};
    use bcr_common::{core, core_tests, wallet::Token};
    use std::str::FromStr;

    /// The exported fee token is a V5 token that carries this mint's identity, and
    /// every proof the vault held survives the round trip through it
    #[tokio::test]
    async fn generate_token_round_trips_the_unspent_proofs() {
        let (keyset_info, keyset) = core_tests::generate_random_ecash_keyset();
        let proofs = core_tests::generate_random_ecash_proofs(
            &keyset,
            &[cashu::Amount::from(1_u64), cashu::Amount::from(8_u64)],
        );
        let mint_id = NodeId::new(
            core::generate_random_keypair().public_key(),
            bitcoin::Network::Regtest,
        );
        let unit = cashu::CurrencyUnit::Custom(String::from("crsat"));
        let mut repo = MockRepository::new();
        let mut wdc_cl = MockWildcatClient::new();
        let ys = proofs.ys().unwrap();
        repo.expect_list_ys()
            .times(1)
            .returning(move || Ok(ys.clone()));
        wdc_cl.expect_check_spent().times(1).returning(|ys| {
            Ok(ys
                .into_iter()
                .map(|y| cashu::ProofState {
                    y,
                    state: cashu::State::Unspent,
                    witness: None,
                })
                .collect())
        });
        repo.expect_delete_proofs().times(1).returning(|_| Ok(()));
        let loaded = proofs.clone();
        repo.expect_load_proofs()
            .times(1)
            .returning(move |_| Ok(loaded.clone()));
        let expected_unit = unit.clone();
        wdc_cl
            .expect_unit()
            .returning(move || expected_unit.clone());
        let service = Service {
            repo: Box::new(repo),
            wdc_cl: Box::new(wdc_cl),
            my_url: cashu::MintUrl::from_str("http://localhost:4343").unwrap(),
            mint_id: mint_id.clone(),
        };
        let now = chrono::Utc::now();
        let token = service.generate_token(now).await.unwrap();
        let encoded = token.to_string();
        assert!(encoded.starts_with("bitcrrC"), "{encoded}");
        assert_eq!(Token::from_str(&encoded).unwrap(), token);
        assert_eq!(token.mint_id(), Some(&mint_id));
        assert_eq!(token.network(), Some(bitcoin::Network::Regtest));
        assert_eq!(token.unit(), Some(unit));
        assert_eq!(
            token.mint_url().unwrap().to_string(),
            "http://localhost:4343"
        );
        assert_eq!(
            token.memo().as_deref(),
            Some(format!("Treasury token generated at {now}").as_str())
        );
        assert_eq!(token.value().unwrap(), cashu::Amount::from(9_u64));
        let mut recovered: Vec<cashu::Proof> = token
            .proofs(&[cashu::KeySetInfo::from(keyset_info).into()])
            .unwrap()
            .into_iter()
            .map(Into::into)
            .collect();
        let mut expected = proofs;
        recovered.sort_by(|a, b| a.secret.cmp(&b.secret));
        expected.sort_by(|a, b| a.secret.cmp(&b.secret));
        assert_eq!(recovered, expected);
    }
}
