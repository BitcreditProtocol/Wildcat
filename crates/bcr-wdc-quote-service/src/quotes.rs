// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use bcr_ebill_core::contact::IdentityPublicData;
use cashu::nuts::nut00 as cdk00;
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::error::{Error, Result};
use crate::TStamp;

#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BillInfo {
    pub id: String,
    pub drawee: IdentityPublicData,
    pub drawer: IdentityPublicData,
    pub payee: IdentityPublicData,
    pub endorsees: Vec<IdentityPublicData>,
    pub sum: u64,
    pub maturity_date: TStamp,
}
impl TryFrom<bcr_wdc_webapi::quotes::BillInfo> for BillInfo {
    type Error = Error;
    fn try_from(bill: bcr_wdc_webapi::quotes::BillInfo) -> Result<Self> {
        let maturity_date = TStamp::from_str(&bill.maturity_date).map_err(Error::Chrono)?;
        Ok(Self {
            id: bill.id,
            drawee: bill.drawee.into(),
            drawer: bill.drawer.into(),
            payee: bill.payee.into(),
            endorsees: bill.endorsees.into_iter().map(Into::into).collect(),
            sum: bill.sum,
            maturity_date,
        })
    }
}
impl From<BillInfo> for bcr_wdc_webapi::quotes::BillInfo {
    fn from(bill: BillInfo) -> Self {
        let maturity_date = bill.maturity_date.to_rfc3339();
        Self {
            id: bill.id,
            drawee: bill.drawee.into(),
            drawer: bill.drawer.into(),
            payee: bill.payee.into(),
            endorsees: bill.endorsees.into_iter().map(Into::into).collect(),
            sum: bill.sum,
            maturity_date,
        }
    }
}

#[derive(Debug, Clone, strum::EnumDiscriminants)]
#[strum_discriminants(derive(serde::Serialize))]
pub enum QuoteStatus {
    Pending {
        blinds: Vec<cdk00::BlindedMessage>,
    },
    Denied,
    Offered {
        signatures: Vec<cdk00::BlindSignature>,
        ttl: TStamp,
    },
    Rejected {
        tstamp: TStamp,
    },
    Accepted {
        signatures: Vec<cdk00::BlindSignature>,
    },
}

#[derive(Debug, Clone)]
pub struct Quote {
    pub status: QuoteStatus,
    pub id: Uuid,
    pub bill: BillInfo,
    pub submitted: TStamp,
}

pub struct LightQuote {
    pub id: Uuid,
    pub status: QuoteStatusDiscriminants,
    pub sum: u64,
}

impl Quote {
    pub fn new(bill: BillInfo, blinds: Vec<cdk00::BlindedMessage>, submitted: TStamp) -> Self {
        Self {
            status: QuoteStatus::Pending { blinds },
            id: Uuid::new_v4(),
            bill,
            submitted,
        }
    }

    pub fn deny(&mut self) -> Result<()> {
        if let QuoteStatus::Pending { .. } = self.status {
            self.status = QuoteStatus::Denied;
            Ok(())
        } else {
            Err(Error::QuoteAlreadyResolved(self.id))
        }
    }

    pub fn offer(&mut self, signatures: Vec<cdk00::BlindSignature>, ttl: TStamp) -> Result<()> {
        let QuoteStatus::Pending { .. } = self.status else {
            return Err(Error::QuoteAlreadyResolved(self.id));
        };

        self.status = QuoteStatus::Offered { signatures, ttl };
        Ok(())
    }

    pub fn reject(&mut self, tstamp: TStamp) -> Result<()> {
        if let QuoteStatus::Offered { .. } = self.status {
            self.status = QuoteStatus::Rejected { tstamp };
            Ok(())
        } else {
            Err(Error::QuoteAlreadyResolved(self.id))
        }
    }

    pub fn accept(&mut self) -> Result<()> {
        if let QuoteStatus::Offered { signatures, .. } = &self.status {
            self.status = QuoteStatus::Accepted {
                signatures: signatures.clone(),
            };
            Ok(())
        } else {
            Err(Error::QuoteAlreadyResolved(self.id))
        }
    }
}
