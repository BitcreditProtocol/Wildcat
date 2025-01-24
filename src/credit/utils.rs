// ----- standard library imports
// ----- extra library imports
use cdk::nuts::nut00 as cdk00;
// ----- local modules
// ----- local imports

pub fn select_blinds_to_target(
    mut target: cdk::Amount,
    blinds: &mut [cdk00::BlindedMessage],
) -> &[cdk00::BlindedMessage] {
    blinds.sort_by_key(|blind| std::cmp::Reverse(blind.amount));
    let marked_end_idx = blinds.partition_point(|blind| blind.amount > cdk::Amount::ZERO);
    let mut selected_idx: usize = 0;
    // first we sign whatever marked blinds user sent
    for idx in 0..marked_end_idx {
        if blinds[idx].amount <= target {
            blinds.swap(selected_idx, idx);
            target -= blinds[selected_idx].amount;
            selected_idx += 1;
            if target == cdk::Amount::ZERO {
                break;
            }
        }
    }
    for target_split in target.split() {
        for idx in selected_idx..blinds.len() {
            if blinds[idx].amount == target_split {
                blinds.swap(selected_idx, idx);
                selected_idx += 1;
                break;
            } else if blinds[idx].amount == cdk::Amount::ZERO {
                blinds.swap(selected_idx, idx);
                blinds[selected_idx].amount = target_split;
                selected_idx += 1;
                break;
            }
        }
    }
    &blinds[0..selected_idx]
}

pub fn calculate_default_expiration_date_for_quote(now: super::TStamp) -> super::TStamp {
    now + chrono::Duration::days(2)
}

#[cfg(test)]
mod tests {

    use super::*;
    use cdk::nuts::nut01 as cdk01;
    use cdk::nuts::nut02 as cdk02;

    const RANDOMS: [&str; 6] = [
        "0244e4420934530b2bdf5161f4c88b3c4f923db158741da51f3bb22b579495862e",
        "03244bce3f2ea7b12acd2004a6c629acf9d01e7eceadfd7f4ce6f7a09134a84474",
        "0212612cddd9e1aa368c500654538c71ebdf70d5bc4a1b642f9c963269505514cc",
        "0292abc8e9eb2935f0ae6fcf7c491ea124a5860ed954e339a0b7f549cd8c190500",
        "02cc8e0448596f0aaec2c62ef02e5a36f53a4e8b7d5a9e906d2c1f8d5cd738ccae",
        "027a238c992c4a5ea59502b2d6b52e6466bf2a775191cbfaf29b9311e8352d99dc",
    ];
    fn publics() -> Vec<cdk01::PublicKey> {
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
                amount: cdk::Amount::from(16_u64),
                blinded_secret: publics[0],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(8_u64),
                blinded_secret: publics[1],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(32_u64),
                blinded_secret: publics[2],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
        ];
        let target = cdk::Amount::from(6_u64);
        let selected = select_blinds_to_target(target, &mut blinds);
        assert_eq!(selected.len(), 0);
    }

    #[test]
    fn test_select_blind_signatures_all_blanks() {
        let publics = publics();
        let mut blinds = vec![
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(0_u64),
                blinded_secret: publics[0],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(0_u64),
                blinded_secret: publics[1],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(0_u64),
                blinded_secret: publics[2],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
        ];
        let target = cdk::Amount::from(6_u64);
        let selected = select_blinds_to_target(target, &mut blinds);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].amount, cdk::Amount::from(4_u64));
        assert_eq!(selected[0].blinded_secret.to_hex(), RANDOMS[0]);
        assert_eq!(selected[1].amount, cdk::Amount::from(2_u64));
        assert_eq!(selected[1].blinded_secret.to_hex(), RANDOMS[1]);
    }

    #[test]
    fn test_select_blind_signatures_all_marked_blinds() {
        let publics = publics();
        let mut blinds = vec![
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(1),
                blinded_secret: publics[0],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(16_u64),
                blinded_secret: publics[1],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(2_u64),
                blinded_secret: publics[2],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(4_u64),
                blinded_secret: publics[3],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
        ];
        let target = cdk::Amount::from(6_u64);
        let selected = select_blinds_to_target(target, &mut blinds);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].amount, cdk::Amount::from(4_u64));
        assert_eq!(selected[0].blinded_secret.to_hex(), RANDOMS[3]);
        assert_eq!(selected[1].amount, cdk::Amount::from(2_u64));
        assert_eq!(selected[1].blinded_secret.to_hex(), RANDOMS[2]);
    }

    #[test]
    fn test_select_blind_signatures_marked_and_blanks() {
        let publics = publics();
        let mut blinds = vec![
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(0),
                blinded_secret: publics[0],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(16_u64),
                blinded_secret: publics[1],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(2_u64),
                blinded_secret: publics[2],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(4_u64),
                blinded_secret: publics[3],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
        ];
        let target = cdk::Amount::from(6_u64);
        let selected = select_blinds_to_target(target, &mut blinds);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].amount, cdk::Amount::from(4_u64));
        assert_eq!(selected[0].blinded_secret.to_hex(), RANDOMS[3]);
        assert_eq!(selected[1].amount, cdk::Amount::from(2_u64));
        assert_eq!(selected[1].blinded_secret.to_hex(), RANDOMS[2]);
    }

    #[test]
    fn test_select_blind_signatures_unconventional_split() {
        let publics = publics();
        let mut blinds = vec![
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(1),
                blinded_secret: publics[0],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(1_u64),
                blinded_secret: publics[1],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(0_u64),
                blinded_secret: publics[2],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
            cdk00::BlindedMessage {
                amount: cdk::Amount::from(4_u64),
                blinded_secret: publics[3],
                keyset_id: cdk02::Id::from_bytes(&[0u8; 8]).unwrap(),
                witness: None,
            },
        ];
        let target = cdk::Amount::from(6_u64);
        let selected = select_blinds_to_target(target, &mut blinds);
        assert_eq!(selected.len(), 3);
        assert_eq!(selected[0].amount, cdk::Amount::from(4_u64));
        assert_eq!(selected[0].blinded_secret.to_hex(), RANDOMS[3]);
        assert_eq!(selected[1].amount, cdk::Amount::from(1_u64));
        assert_eq!(selected[1].blinded_secret.to_hex(), RANDOMS[0]);
        assert_eq!(selected[2].amount, cdk::Amount::from(1_u64));
        assert_eq!(selected[2].blinded_secret.to_hex(), RANDOMS[1]);
    }
}
