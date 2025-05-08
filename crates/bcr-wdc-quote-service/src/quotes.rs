// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use bcr_ebill_core::contact::IdentityPublicData;
use bitcoin::Amount;
use cashu::{nut01 as cdk01, nut02 as cdk02};
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
    pub current_holder: IdentityPublicData,
    pub sum: Amount,
    pub maturity_date: TStamp,
}
impl TryFrom<bcr_wdc_webapi::quotes::BillInfo> for BillInfo {
    type Error = Error;
    fn try_from(bill: bcr_wdc_webapi::quotes::BillInfo) -> Result<Self> {
        let maturity_date = TStamp::from_str(&bill.maturity_date).map_err(Error::Chrono)?;
        let current_holder = bill.endorsees.last().unwrap_or(&bill.payee).clone();
        Ok(Self {
            id: bill.id,
            drawee: bill.drawee.into(),
            drawer: bill.drawer.into(),
            payee: bill.payee.into(),
            endorsees: bill.endorsees.into_iter().map(Into::into).collect(),
            current_holder: current_holder.into(),
            sum: Amount::from_sat(bill.sum),
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
            sum: bill.sum.to_sat(),
            maturity_date,
        }
    }
}

#[derive(Debug, Clone, strum::EnumDiscriminants, serde::Serialize, serde::Deserialize)]
#[strum_discriminants(derive(serde::Serialize, serde::Deserialize))]
#[serde(tag = "status")]
pub enum QuoteStatus {
    Pending { public_key: cdk01::PublicKey },
    Denied,
    Offered { keyset_id: cdk02::Id, ttl: TStamp },
    Rejected { tstamp: TStamp },
    Accepted { keyset_id: cdk02::Id },
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
    pub sum: Amount,
}

impl Quote {
    pub fn new(bill: BillInfo, public_key: cdk01::PublicKey, submitted: TStamp) -> Self {
        Self {
            status: QuoteStatus::Pending { public_key },
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

    pub fn offer(&mut self, keyset_id: cdk02::Id, ttl: TStamp) -> Result<()> {
        let QuoteStatus::Pending { .. } = self.status else {
            return Err(Error::QuoteAlreadyResolved(self.id));
        };

        self.status = QuoteStatus::Offered { keyset_id, ttl };
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
        match self.status {
            QuoteStatus::Offered { keyset_id, .. } => {
                self.status = QuoteStatus::Accepted { keyset_id }
            }
            _ => return Err(Error::QuoteAlreadyResolved(self.id)),
        };
        Ok(())
    }
}
