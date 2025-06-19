// ----- standard library imports
use std::str::FromStr;
// ----- extra library imports
use bcr_ebill_core::contact::{BillIdentParticipant, BillParticipant};
use bitcoin::Amount;
use cashu::{nut01 as cdk01, nut02 as cdk02};
use uuid::Uuid;
// ----- local modules
// ----- local imports
use crate::error::{Error, Result};
use crate::TStamp;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BillInfo {
    pub id: String,
    pub drawee: BillIdentParticipant,
    pub drawer: BillIdentParticipant,
    pub payee: BillParticipant,
    pub endorsees: Vec<BillParticipant>,
    pub current_holder: BillParticipant,
    pub sum: Amount,
    pub maturity_date: TStamp,
    pub file_urls: Vec<url::Url>,
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
            file_urls: bill.file_urls,
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
            file_urls: bill.file_urls,
        }
    }
}

#[derive(Debug, Clone, strum::EnumDiscriminants, serde::Serialize, serde::Deserialize)]
#[strum_discriminants(derive(serde::Serialize, serde::Deserialize))]
#[serde(tag = "status")]
pub enum Status {
    Pending {
        public_key: cdk01::PublicKey,
    },
    Canceled {
        tstamp: TStamp,
    },
    Denied {
        tstamp: TStamp,
    },
    Offered {
        keyset_id: cdk02::Id,
        ttl: TStamp,
        discounted: bitcoin::Amount,
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
        keyset_id: cdk02::Id,
    },
}

#[derive(Debug, Clone)]
pub struct Quote {
    pub status: Status,
    pub id: Uuid,
    pub bill: BillInfo,
    pub submitted: TStamp,
}

pub struct LightQuote {
    pub id: Uuid,
    pub status: StatusDiscriminants,
    pub sum: Amount,
    pub maturity_date: TStamp,
}

impl Quote {
    pub fn new(bill: BillInfo, public_key: cdk01::PublicKey, submitted: TStamp) -> Self {
        Self {
            status: Status::Pending { public_key },
            id: Uuid::new_v4(),
            bill,
            submitted,
        }
    }

    pub fn cancel(&mut self, tstamp: TStamp) -> Result<()> {
        if let Status::Pending { .. } = self.status {
            self.status = Status::Canceled { tstamp };
            Ok(())
        } else {
            Err(Error::QuoteAlreadyResolved(self.id))
        }
    }

    pub fn deny(&mut self, tstamp: TStamp) -> Result<()> {
        if let Status::Pending { .. } = self.status {
            self.status = Status::Denied { tstamp };
            Ok(())
        } else {
            Err(Error::QuoteAlreadyResolved(self.id))
        }
    }

    pub fn offer(
        &mut self,
        keyset_id: cdk02::Id,
        ttl: TStamp,
        discounted: bitcoin::Amount,
    ) -> Result<()> {
        let Status::Pending { .. } = self.status else {
            return Err(Error::QuoteAlreadyResolved(self.id));
        };

        self.status = Status::Offered {
            keyset_id,
            ttl,
            discounted,
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
            Err(Error::QuoteAlreadyResolved(self.id))
        }
    }

    pub fn accept(&mut self, tstamp: TStamp) -> Result<()> {
        self.check_expire(tstamp);
        match self.status {
            Status::Offered {
                keyset_id,
                discounted,
                ..
            } => {
                self.status = Status::Accepted {
                    keyset_id,
                    discounted,
                }
            }
            _ => return Err(Error::QuoteAlreadyResolved(self.id)),
        };
        Ok(())
    }
}
