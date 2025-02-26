// ----- standard library imports
// ----- extra library imports
use cashu::nuts::nut00 as cdk00;
use cashu::Amount as cdk_Amount;
// ----- local modules
// ----- local imports

pub fn select_blinds_to_target(
    mut target: cdk_Amount,
    blinds: &mut [cdk00::BlindedMessage],
) -> &[cdk00::BlindedMessage] {
    for (idx, blind) in blinds.iter_mut().enumerate() {
        if target == cdk_Amount::ZERO {
            return &blinds[0..idx];
        }
        if blind.amount == cdk_Amount::ZERO {
            blind.amount = *target.split().first().expect("target > 0"); // split() returns from
                                                                         // highest to lowest
            target -= blind.amount;
        } else if blind.amount <= target {
            target -= blind.amount;
        } else {
            return &blinds[0..idx];
        }
    }
    blinds
}

pub fn calculate_default_expiration_date_for_quote(now: crate::TStamp) -> super::TStamp {
    now + chrono::Duration::days(2)
}

#[cfg(test)]
pub mod tests {

    use super::*;
    use cashu::nuts::nut01 as cdk01;
    use cashu::nuts::nut02 as cdk02;

    pub const RANDOMS: [&str; 6] = [
        "0244e4420934530b2bdf5161f4c88b3c4f923db158741da51f3bb22b579495862e",
        "03244bce3f2ea7b12acd2004a6c629acf9d01e7eceadfd7f4ce6f7a09134a84474",
        "0212612cddd9e1aa368c500654538c71ebdf70d5bc4a1b642f9c963269505514cc",
        "0292abc8e9eb2935f0ae6fcf7c491ea124a5860ed954e339a0b7f549cd8c190500",
        "02cc8e0448596f0aaec2c62ef02e5a36f53a4e8b7d5a9e906d2c1f8d5cd738ccae",
        "027a238c992c4a5ea59502b2d6b52e6466bf2a775191cbfaf29b9311e8352d99dc",
    ];
    pub fn publics() -> Vec<cdk01::PublicKey> {
        RANDOMS
            .iter()
            .map(|key| cdk01::PublicKey::from_hex(key).unwrap())
            .collect()
    }
    #[test]
    fn test_select_blind_signatures_no_valid_blinds() {
        let publics = publics();
        let mut blinds = vec![
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(16_u64),
                blinded_secret: publics[0],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(8_u64),
                blinded_secret: publics[1],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(32_u64),
                blinded_secret: publics[2],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
        ];
        let target = cdk_Amount::from(6_u64);
        let selected = select_blinds_to_target(target, &mut blinds);
        assert_eq!(selected.len(), 0);
    }

    #[test]
    fn test_select_blind_signatures_all_blanks() {
        let publics = publics();
        let mut blinds = vec![
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(0_u64),
                blinded_secret: publics[0],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(0_u64),
                blinded_secret: publics[1],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(0_u64),
                blinded_secret: publics[2],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
        ];
        let target = cdk_Amount::from(6_u64);
        let selected = select_blinds_to_target(target, &mut blinds);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].amount, cdk_Amount::from(4_u64));
        assert_eq!(selected[0].blinded_secret.to_hex(), RANDOMS[0]);
        assert_eq!(selected[1].amount, cdk_Amount::from(2_u64));
        assert_eq!(selected[1].blinded_secret.to_hex(), RANDOMS[1]);
    }

    #[test]
    fn test_select_blind_signatures_all_marked_blinds() {
        let publics = publics();
        let mut blinds = vec![
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(16_u64),
                blinded_secret: publics[1],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(4_u64),
                blinded_secret: publics[3],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(2_u64),
                blinded_secret: publics[2],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(1),
                blinded_secret: publics[0],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
        ];
        let target = cdk_Amount::from(6_u64);
        let selected = select_blinds_to_target(target, &mut blinds);
        assert_eq!(selected.len(), 0);
    }

    #[test]
    fn test_select_blind_signatures_marked_and_blanks() {
        let publics = publics();
        let mut blinds = vec![
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(4_u64),
                blinded_secret: publics[3],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(2_u64),
                blinded_secret: publics[2],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(0),
                blinded_secret: publics[0],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
        ];
        let target = cdk_Amount::from(6_u64);
        let selected = select_blinds_to_target(target, &mut blinds);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].amount, cdk_Amount::from(4_u64));
        assert_eq!(selected[0].blinded_secret.to_hex(), RANDOMS[3]);
        assert_eq!(selected[1].amount, cdk_Amount::from(2_u64));
        assert_eq!(selected[1].blinded_secret.to_hex(), RANDOMS[2]);
    }

    #[test]
    fn test_select_blind_signatures_unconventional_split() {
        let publics = publics();
        let mut blinds = vec![
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(4_u64),
                blinded_secret: publics[3],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(1),
                blinded_secret: publics[0],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(1_u64),
                blinded_secret: publics[1],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk_Amount::from(0_u64),
                blinded_secret: publics[2],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
        ];
        let target = cdk_Amount::from(6_u64);
        let selected = select_blinds_to_target(target, &mut blinds);
        assert_eq!(selected.len(), 3);
        assert_eq!(selected[0].amount, cdk_Amount::from(4_u64));
        assert_eq!(selected[0].blinded_secret.to_hex(), RANDOMS[3]);
        assert_eq!(selected[1].amount, cdk_Amount::from(1_u64));
        assert_eq!(selected[1].blinded_secret.to_hex(), RANDOMS[0]);
        assert_eq!(selected[2].amount, cdk_Amount::from(1_u64));
        assert_eq!(selected[2].blinded_secret.to_hex(), RANDOMS[1]);
    }
}
