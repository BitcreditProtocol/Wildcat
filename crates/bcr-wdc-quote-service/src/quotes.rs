// ----- standard library imports
// ----- extra library imports
#[cfg(test)]
use arbitrary::Arbitrary;
use bcr_common::{cashu, core::BillId, wire::quotes as wire_quotes};
#[cfg(test)]
use bcr_common::{core_tests::random_bill_id, wire_tests::random_identity_public_data};
use bcr_ebill_core::protocol::blockchain::bill::participant::{
    BillIdentParticipant, BillParticipant,
};
use bcr_wdc_utils::convert;
use bitcoin::Amount;
use uuid::Uuid;
// ----- local imports
use crate::error::{Error, Result};
use crate::TStamp;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BillInfo {
    pub id: BillId,
    pub drawee: BillIdentParticipant,
    pub drawer: BillIdentParticipant,
    pub payee: BillParticipant,
    pub endorsees: Vec<BillParticipant>,
    pub current_holder: BillParticipant,
    pub sum: Amount,
    pub maturity_date: chrono::NaiveDate,
    pub file_urls: Vec<url::Url>,
    pub shared_bill_data: String, // The base58 encoded, encrypted, borshed BillBlockPlaintextWrappers of the bill
}
pub fn convert_to_billinfo(
    bill: wire_quotes::BillInfo,
    shared_bill: wire_quotes::SharedBill,
) -> Result<BillInfo> {
    let maturity_date = bill.maturity_date;
    let current_holder = bill.endorsees.last().unwrap_or(&bill.payee).clone();
    Ok(BillInfo {
        id: bill.id,
        drawee: convert::billidentparticipant_wire2ebill(bill.drawee)?,
        drawer: convert::billidentparticipant_wire2ebill(bill.drawer)?,
        payee: convert::billparticipant_wire2ebill(bill.payee)?,
        endorsees: bill
            .endorsees
            .into_iter()
            .map(convert::billparticipant_wire2ebill)
            .collect::<std::result::Result<_, convert::Error>>()?,
        current_holder: convert::billparticipant_wire2ebill(current_holder)?,
        sum: Amount::from_sat(bill.sum),
        maturity_date,
        file_urls: bill.file_urls,
        shared_bill_data: shared_bill.data,
    })
}
impl From<BillInfo> for wire_quotes::BillInfo {
    fn from(bill: BillInfo) -> Self {
        Self {
            id: bill.id,
            drawee: convert::billidentparticipant_ebill2wire(bill.drawee),
            drawer: convert::billidentparticipant_ebill2wire(bill.drawer),
            payee: convert::billparticipant_ebill2wire(bill.payee),
            endorsees: bill
                .endorsees
                .into_iter()
                .map(convert::billparticipant_ebill2wire)
                .collect(),
            sum: bill.sum.to_sat(),
            maturity_date: bill.maturity_date,
            file_urls: bill.file_urls,
        }
    }
}

#[cfg(test)]
impl BillInfo {
    pub fn random() -> Self {
        let seed: [u8; 4] = rand::random();
        let mut seed = arbitrary::Unstructured::new(&seed);
        Self {
            id: random_bill_id(),
            drawee: convert::billidentparticipant_wire2ebill(random_identity_public_data().1)
                .unwrap(),
            drawer: convert::billidentparticipant_wire2ebill(random_identity_public_data().1)
                .unwrap(),
            payee: BillParticipant::Ident(
                convert::billidentparticipant_wire2ebill(random_identity_public_data().1).unwrap(),
            ),
            endorsees: Vec::default(),
            current_holder: BillParticipant::Ident(
                convert::billidentparticipant_wire2ebill(random_identity_public_data().1).unwrap(),
            ),
            sum: bitcoin::Amount::arbitrary(&mut seed).unwrap(),
            maturity_date: chrono::NaiveDate::default(),
            file_urls: Vec::default(),
            shared_bill_data: String::default(),
        }
    }
}

#[derive(Debug, Clone, strum::EnumDiscriminants, serde::Serialize, serde::Deserialize)]
#[strum_discriminants(
    derive(
        serde::Serialize,
        serde::Deserialize,
        strum::Display,
        strum::EnumString
    ),
    strum(serialize_all = "lowercase")
)]
#[serde(tag = "status")]
pub enum Status {
    Pending {
        wallet_pubkey: cashu::PublicKey,
    },
    Canceled {
        tstamp: TStamp,
    },
    Denied {
        tstamp: TStamp,
    },
    Offered {
        keyset_id: cashu::Id,
        ttl: TStamp,
        discounted: bitcoin::Amount,
        wallet_pubkey: cashu::PublicKey,
    },
    OfferExpired {
        discounted: bitcoin::Amount,
        tstamp: TStamp,
    },
    Rejected {
        discounted: bitcoin::Amount,
        tstamp: TStamp,
    },
    Accepted {
        discounted: bitcoin::Amount,
        keyset_id: cashu::Id,
        wallet_pubkey: cashu::PublicKey,
    },
    MintingEnabled {
        keyset_id: cashu::Id,
        wallet_pubkey: cashu::PublicKey,
        discounted: bitcoin::Amount,
        fee: cashu::Amount,
    },
    FailedEbillValidation {
        discounted: bitcoin::Amount,
        keyset_id: cashu::Id,
        wallet_pubkey: cashu::PublicKey,
    },
}

#[derive(Debug, Clone)]
pub struct Quote {
    pub status: Status,
    pub id: Uuid,
    pub bill: BillInfo,
    pub submitted: TStamp,
    pub(crate) credit_program: Option<CreditProgramBinding>,
    pub(crate) authorization_receipt: Option<wire_quotes::CreditAuthorizationReceipt>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CreditProgramBinding {
    version: String,
    digest: String,
}

impl<'de> serde::Deserialize<'de> for CreditProgramBinding {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct StoredCreditProgramBinding {
            version: String,
            digest: String,
        }

        let stored = StoredCreditProgramBinding::deserialize(deserializer)?;
        Self::new(stored.version, stored.digest).map_err(serde::de::Error::custom)
    }
}

impl CreditProgramBinding {
    pub fn new(version: String, digest: String) -> Result<Self> {
        if version.trim() != version
            || version.is_empty()
            || version.len() > 128
            || version.chars().any(char::is_control)
        {
            return Err(Error::InvalidInput(String::from(
                "credit program version must contain 1 to 128 non-control, non-whitespace-padded characters",
            )));
        }
        let Some(hex) = digest.strip_prefix("sha256:") else {
            return Err(Error::InvalidInput(String::from(
                "credit program digest must use sha256:<64 lowercase hex characters>",
            )));
        };
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Error::InvalidInput(String::from(
                "credit program digest must use sha256:<64 lowercase hex characters>",
            )));
        }
        Ok(Self { version, digest })
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }
}

#[cfg(test)]
pub(crate) fn test_credit_program_binding() -> CreditProgramBinding {
    CreditProgramBinding::new(
        String::from("test-credit-program-v1"),
        String::from("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
    )
    .expect("valid test credit program binding")
}

pub struct LightQuote {
    pub id: Uuid,
    pub status: StatusDiscriminants,
    pub sum: Amount,
    pub maturity_date: chrono::NaiveDate,
}

impl Quote {
    pub fn new(
        bill: BillInfo,
        wallet_pubkey: cashu::PublicKey,
        submitted: TStamp,
        credit_program: CreditProgramBinding,
    ) -> Self {
        Self {
            status: Status::Pending { wallet_pubkey },
            id: Uuid::new_v4(),
            bill,
            submitted,
            credit_program: Some(credit_program),
            authorization_receipt: None,
        }
    }

    pub(crate) fn new_with_id(
        id: Uuid,
        bill: BillInfo,
        wallet_pubkey: cashu::PublicKey,
        submitted: TStamp,
        credit_program: CreditProgramBinding,
    ) -> Self {
        Self {
            status: Status::Pending { wallet_pubkey },
            id,
            bill,
            submitted,
            credit_program: Some(credit_program),
            authorization_receipt: None,
        }
    }

    pub fn credit_program(&self) -> Option<&CreditProgramBinding> {
        self.credit_program.as_ref()
    }

    pub fn authorization_receipt(&self) -> Option<&wire_quotes::CreditAuthorizationReceipt> {
        self.authorization_receipt.as_ref()
    }

    pub(crate) fn require_credit_program(&self) -> Result<()> {
        self.credit_program
            .as_ref()
            .map(|_| ())
            .ok_or(Error::CreditProgramNotBound(self.id))
    }

    fn require_credit_authorization(&self) -> Result<()> {
        let receipt = self
            .authorization_receipt
            .as_ref()
            .ok_or(Error::CreditAuthorizationRequired)?;
        let expected_result_digest = match &self.status {
            Status::Offered {
                discounted, ttl, ..
            } => crate::authorization::offer_result_digest(self.id, *discounted, *ttl),
            _ => return Err(Error::CreditAuthorizationInvalid),
        };
        if receipt.receipt_version != "credit-authorization-receipt-v1"
            || receipt.status != "completed"
            || receipt.action != crate::authorization::AUTHORIZATION_ACTION
            || receipt.effect_id != self.id.to_string()
            || receipt.bill_id != self.bill.id.to_string()
            || receipt.result_digest != expected_result_digest
            || !receipt.synthetic
        {
            return Err(Error::CreditAuthorizationInvalid);
        }
        Ok(())
    }

    pub fn cancel(&mut self, tstamp: TStamp) -> Result<()> {
        if let Status::Pending { .. } = self.status {
            self.status = Status::Canceled { tstamp };
            Ok(())
        } else {
            Err(Error::InvalidQuoteStatus(
                self.id,
                StatusDiscriminants::Pending,
                StatusDiscriminants::from(self.status.clone()),
            ))
        }
    }

    pub fn deny(&mut self, tstamp: TStamp) -> Result<()> {
        if self.credit_program.is_some() {
            return Err(Error::CreditAuthorizationRequired);
        }
        if let Status::Pending { .. } = self.status {
            self.status = Status::Denied { tstamp };
            Ok(())
        } else {
            Err(Error::InvalidQuoteStatus(
                self.id,
                StatusDiscriminants::Pending,
                StatusDiscriminants::from(self.status.clone()),
            ))
        }
    }

    pub fn offer(
        &mut self,
        keyset_id: cashu::Id,
        ttl: TStamp,
        discounted: bitcoin::Amount,
    ) -> Result<()> {
        self.require_credit_program()?;
        let Status::Pending { wallet_pubkey, .. } = self.status else {
            return Err(Error::InvalidQuoteStatus(
                self.id,
                StatusDiscriminants::Pending,
                StatusDiscriminants::from(self.status.clone()),
            ));
        };

        self.status = Status::Offered {
            keyset_id,
            ttl,
            discounted,
            wallet_pubkey,
        };
        Ok(())
    }

    pub fn check_expire(&mut self, tstamp: TStamp) -> bool {
        if let Status::Offered {
            ttl, discounted, ..
        } = self.status
        {
            if tstamp > ttl {
                self.status = Status::OfferExpired {
                    tstamp: ttl,
                    discounted,
                };
                return true;
            }
        }
        false
    }

    pub fn reject(&mut self, tstamp: TStamp) -> Result<()> {
        if let Status::Offered { discounted, .. } = self.status {
            self.status = Status::Rejected { tstamp, discounted };
            Ok(())
        } else {
            Err(Error::InvalidQuoteStatus(
                self.id,
                StatusDiscriminants::Offered,
                StatusDiscriminants::from(self.status.clone()),
            ))
        }
    }

    pub fn accept(&mut self, tstamp: TStamp) -> Result<()> {
        self.require_credit_program()?;
        self.require_credit_authorization()?;
        self.check_expire(tstamp);
        match self.status {
            Status::Offered {
                keyset_id,
                discounted,
                wallet_pubkey,
                ..
            } => {
                self.status = Status::Accepted {
                    keyset_id,
                    discounted,
                    wallet_pubkey,
                }
            }
            _ => {
                return Err(Error::InvalidQuoteStatus(
                    self.id,
                    StatusDiscriminants::Offered,
                    StatusDiscriminants::from(self.status.clone()),
                ))
            }
        };
        Ok(())
    }

    pub fn override_failed_ebill_validation(&mut self, fee: cashu::Amount) -> Result<()> {
        self.require_credit_program()?;
        match self.status {
            Status::FailedEbillValidation {
                keyset_id,
                wallet_pubkey,
                discounted,
            } => {
                self.status = Status::MintingEnabled {
                    keyset_id,
                    wallet_pubkey,
                    discounted,
                    fee,
                }
            }
            _ => {
                return Err(Error::InvalidQuoteStatus(
                    self.id,
                    StatusDiscriminants::FailedEbillValidation,
                    StatusDiscriminants::from(self.status.clone()),
                ))
            }
        };
        Ok(())
    }

    pub fn start_minting(&mut self, fee: cashu::Amount) -> Result<()> {
        self.require_credit_program()?;
        match self.status {
            Status::Accepted {
                keyset_id,
                wallet_pubkey,
                discounted,
            } => {
                self.status = Status::MintingEnabled {
                    keyset_id,
                    wallet_pubkey,
                    discounted,
                    fee,
                }
            }
            _ => {
                return Err(Error::InvalidQuoteStatus(
                    self.id,
                    StatusDiscriminants::Accepted,
                    StatusDiscriminants::from(self.status.clone()),
                ))
            }
        };
        Ok(())
    }

    pub fn set_failed_ebill_validation(&mut self) -> Result<()> {
        self.require_credit_program()?;
        match self.status {
            Status::Accepted {
                keyset_id,
                wallet_pubkey,
                discounted,
            } => {
                self.status = Status::FailedEbillValidation {
                    keyset_id,
                    wallet_pubkey,
                    discounted,
                }
            }
            _ => {
                return Err(Error::InvalidQuoteStatus(
                    self.id,
                    StatusDiscriminants::Accepted,
                    StatusDiscriminants::from(self.status.clone()),
                ))
            }
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use bcr_wdc_utils::keys::test_utils as keys_test;

    #[test]
    fn credit_program_binding_is_strict() {
        assert!(CreditProgramBinding::new(
            String::from("gt-coffee-v1"),
            String::from("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
        )
        .is_ok());
        assert!(CreditProgramBinding::new(
            String::from(" gt-coffee-v1"),
            String::from("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
        )
        .is_err());
        assert!(CreditProgramBinding::new(
            String::from("gt-coffee\nv1"),
            String::from("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
        )
        .is_err());
        assert!(CreditProgramBinding::new(
            String::from("gt-coffee-v1"),
            String::from("sha256:0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef"),
        )
        .is_err());
    }

    #[test]
    fn malformed_persisted_credit_program_binding_is_rejected() {
        let malformed = serde_json::json!({
            "version": "gt-coffee-v1",
            "digest": "sha256:not-a-digest"
        });

        assert!(serde_json::from_value::<CreditProgramBinding>(malformed).is_err());
    }

    #[test]
    fn legacy_unbound_quote_cannot_be_offered() {
        let mut quote = Quote::new(
            BillInfo::random(),
            keys_test::publics()[0],
            TStamp::default(),
            test_credit_program_binding(),
        );
        quote.credit_program = None;
        let keyset_id = bcr_common::core_tests::generate_random_ecash_keyset().0.id;

        let result = quote.offer(keyset_id, TStamp::default(), bitcoin::Amount::from_sat(1));

        assert!(matches!(result, Err(Error::CreditProgramNotBound(id)) if id == quote.id));
        assert!(matches!(quote.status, Status::Pending { .. }));
    }

    #[test]
    fn legacy_unbound_quote_can_be_denied() {
        let mut quote = Quote::new(
            BillInfo::random(),
            keys_test::publics()[0],
            TStamp::default(),
            test_credit_program_binding(),
        );
        quote.credit_program = None;

        let result = quote.deny(TStamp::default());

        assert!(result.is_ok());
        assert!(matches!(quote.status, Status::Denied { .. }));
    }

    #[test]
    fn governed_quote_rejects_unsigned_denial_without_changing_state() {
        let mut quote = Quote::new(
            BillInfo::random(),
            keys_test::publics()[0],
            TStamp::default(),
            test_credit_program_binding(),
        );

        let result = quote.deny(TStamp::default());

        let error = result.unwrap_err();
        assert!(matches!(&error, Error::CreditAuthorizationRequired));
        assert_eq!(
            error.into_response().status(),
            axum::http::StatusCode::BAD_REQUEST
        );
        assert!(matches!(quote.status, Status::Pending { .. }));
    }

    #[test]
    fn offer_without_authorization_receipt_cannot_be_accepted() {
        let mut quote = Quote::new(
            BillInfo::random(),
            keys_test::publics()[0],
            TStamp::default(),
            test_credit_program_binding(),
        );
        let keyset_id = bcr_common::core_tests::generate_random_ecash_keyset().0.id;
        quote
            .offer(keyset_id, TStamp::default(), bitcoin::Amount::from_sat(1))
            .unwrap();

        assert!(matches!(
            quote.accept(TStamp::default()),
            Err(Error::CreditAuthorizationRequired)
        ));
        assert!(matches!(quote.status, Status::Offered { .. }));
    }

    #[test]
    fn governed_denial_receipt_cannot_authorize_offer_acceptance() {
        let mut quote = Quote::new(
            BillInfo::random(),
            keys_test::publics()[0],
            TStamp::default(),
            test_credit_program_binding(),
        );
        let keyset_id = bcr_common::core_tests::generate_random_ecash_keyset().0.id;
        quote
            .offer(keyset_id, TStamp::default(), bitcoin::Amount::from_sat(1))
            .unwrap();
        quote.authorization_receipt = Some(wire_quotes::CreditAuthorizationReceipt {
            receipt_version: String::from("credit-authorization-receipt-v1"),
            operation_id: format!("sha256:{}", "a".repeat(64)),
            authorization_digest: format!("sha256:{}", "b".repeat(64)),
            case_id: uuid::Uuid::new_v4().to_string(),
            status: String::from("completed"),
            mint_id: String::from("local-wildcat"),
            bill_id: quote.bill.id.to_string(),
            action: String::from(crate::authorization::QUOTE_DENIAL_ACTION),
            effect_id: quote.id.to_string(),
            result_digest: format!("sha256:{}", "c".repeat(64)),
            completed_at: String::from("2026-08-25T12:00:00.000Z"),
            synthetic: true,
        });

        assert!(matches!(
            quote.accept(TStamp::default()),
            Err(Error::CreditAuthorizationInvalid)
        ));
        assert!(matches!(quote.status, Status::Offered { .. }));
    }

    #[test]
    fn tampered_offer_result_receipt_cannot_authorize_acceptance() {
        let mut quote = Quote::new(
            BillInfo::random(),
            keys_test::publics()[0],
            TStamp::default(),
            test_credit_program_binding(),
        );
        let keyset_id = bcr_common::core_tests::generate_random_ecash_keyset().0.id;
        let ttl = chrono::DateTime::parse_from_rfc3339("2026-08-26T12:00:00.000Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let discounted = bitcoin::Amount::from_sat(7_735_000);
        quote.offer(keyset_id, ttl, discounted).unwrap();
        quote.authorization_receipt = Some(wire_quotes::CreditAuthorizationReceipt {
            receipt_version: String::from("credit-authorization-receipt-v1"),
            operation_id: format!("sha256:{}", "a".repeat(64)),
            authorization_digest: format!("sha256:{}", "b".repeat(64)),
            case_id: uuid::Uuid::new_v4().to_string(),
            status: String::from("completed"),
            mint_id: String::from("local-wildcat"),
            bill_id: quote.bill.id.to_string(),
            action: String::from(crate::authorization::AUTHORIZATION_ACTION),
            effect_id: quote.id.to_string(),
            result_digest: format!("sha256:{}", "c".repeat(64)),
            completed_at: String::from("2026-08-25T12:00:00.000Z"),
            synthetic: true,
        });

        assert_ne!(
            quote.authorization_receipt().unwrap().result_digest,
            crate::authorization::offer_result_digest(quote.id, discounted, ttl)
        );
        assert!(matches!(
            quote.accept(TStamp::default()),
            Err(Error::CreditAuthorizationInvalid)
        ));
        assert!(matches!(quote.status, Status::Offered { .. }));
    }
}
