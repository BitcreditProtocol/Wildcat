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
    use bcr_wdc_keys::test_utils as keys_test;
    use cashu::nuts::nut02 as cdk02;

    #[test]
    fn test_select_blind_signatures_no_valid_blinds() {
        let publics = keys_test::publics();
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
        let publics = keys_test::publics();
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
        assert_eq!(selected[0].blinded_secret.to_hex(), keys_test::RANDOMS[0]);
        assert_eq!(selected[1].amount, cdk_Amount::from(2_u64));
        assert_eq!(selected[1].blinded_secret.to_hex(), keys_test::RANDOMS[1]);
    }

    #[test]
    fn test_select_blind_signatures_all_marked_blinds() {
        let publics = keys_test::publics();
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
        let publics = keys_test::publics();
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
        assert_eq!(selected[0].blinded_secret.to_hex(), keys_test::RANDOMS[3]);
        assert_eq!(selected[1].amount, cdk_Amount::from(2_u64));
        assert_eq!(selected[1].blinded_secret.to_hex(), keys_test::RANDOMS[2]);
    }

    #[test]
    fn test_select_blind_signatures_unconventional_split() {
        let publics = keys_test::publics();
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
        assert_eq!(selected[0].blinded_secret.to_hex(), keys_test::RANDOMS[3]);
        assert_eq!(selected[1].amount, cdk_Amount::from(1_u64));
        assert_eq!(selected[1].blinded_secret.to_hex(), keys_test::RANDOMS[0]);
        assert_eq!(selected[2].amount, cdk_Amount::from(1_u64));
        assert_eq!(selected[2].blinded_secret.to_hex(), keys_test::RANDOMS[1]);
    }
}
