
    pub fn generate_proofs(
        keyset: &cdk02::MintKeySet,
        amounts: &[cdk_Amount],
    ) -> Vec<cdk00::Proof> {
        let mut proofs: Vec<cdk00::Proof> = Vec::new();
        for amount in amounts {
            let keypair = keyset.keys.get(amount).expect("keys for amount");
            let secret = cdk_secret::Secret::new(rand::random::<u64>().to_string());
            let (b_, r) =
                cdk_dhke::blind_message(secret.as_bytes(), None).expect("cdk_dhke::blind_message");
            let c_ =
                cdk_dhke::sign_message(&keypair.secret_key, &b_).expect("cdk_dhke::sign_message");
            let c =
                cdk_dhke::unblind_message(&c_, &r, &keypair.public_key).expect("unblind_message");
            proofs.push(cdk00::Proof::new(*amount, keyset.id, secret, c));
        }
        proofs
    }

    pub fn generate_blinds(
        keyset: &cdk02::MintKeySet,
        amounts: &[cdk_Amount],
    ) -> Vec<(cdk00::BlindedMessage, cdk_secret::Secret, cdk01::SecretKey)> {
        let mut blinds: Vec<(cdk00::BlindedMessage, cdk_secret::Secret, cdk01::SecretKey)> =
            Vec::new();
        for amount in amounts {
            let _keypair = keyset.keys.get(amount).expect("keys for amount");
            let secret = cdk_secret::Secret::new(rand::random::<u64>().to_string());
            let (b_, r) =
                cdk_dhke::blind_message(secret.as_bytes(), None).expect("cdk_dhke::blind_message");
            blinds.push((
                cdk00::BlindedMessage::new(*amount, keyset.id, b_),
                secret,
                r,
            ));
        }
        blinds
    }

    pub fn verify_signatures_data(
        keyset: &cdk02::MintKeySet,
        signatures: impl std::iter::IntoIterator<Item = (cdk00::BlindedMessage, cdk00::BlindSignature)>,
    ) -> bool {
        for signature in signatures.into_iter() {
            let msg = signature.0;
            let sig = signature.1;
            if msg.keyset_id != keyset.id || sig.keyset_id != keyset.id {
                return false;
            }
            if msg.amount != sig.amount {
                return false;
            }

            let keypair = keyset.keys.get(&signature.0.amount);
            if keypair.is_none() {
                return false;
            }
        }
        true
    }


