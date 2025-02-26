
#[derive(Default, Clone)]
pub struct ProofMap {
    proofs: Arc<RwLock<HashMap<cdk01::PublicKey, cdk07::ProofState>>>,
}

#[async_trait()]
impl swap::ProofRepository for ProofMap {
    async fn spend(&self, tokens: &[cdk00::Proof]) -> AnyResult<()> {
        let mut writer = self.proofs.write().unwrap();
        for token in tokens {
            let y = cdk_dhke::hash_to_curve(&token.secret.to_bytes())?;
            let proofstate = cdk07::ProofState {
                y,
                state: cdk07::State::Spent,
                witness: None,
            };
            writer.insert(y, proofstate);
        }
        Ok(())
    }

    async fn get_state(&self, tokens: &[cdk00::Proof]) -> AnyResult<Vec<cdk07::State>> {
        let mut states: Vec<cdk07::State> = Vec::new();
        let reader = self.proofs.read().unwrap();
        for token in tokens {
            let y = cdk_dhke::hash_to_curve(&token.secret.to_bytes())?;
            let state = reader.get(&y).map_or(cdk07::State::Unspent, |x| x.state);
            states.push(state);
        }
        Ok(states)
    }
}

