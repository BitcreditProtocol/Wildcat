// ----- standard library imports
// ----- extra library imports
use bcr_common::cashu;
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
        let ys = proofs
            .iter()
            .map(|p| p.y())
            .collect::<std::result::Result<_, _>>()?;
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
        let memo = format!("Treasury token generated at {now}");
        let token = bcr_common::wallet::Token::new_bitcr(
            self.my_url.clone(),
            proofs,
            Some(memo),
            self.wdc_cl.unit(),
        );
        Ok(token)
    }
}
