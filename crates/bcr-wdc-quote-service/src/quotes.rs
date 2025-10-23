// ----- standard library imports
// ----- extra library imports
use bcr_common::{core::BillId, wire::quotes as wire_quotes};
use bcr_ebill_core::contact::{BillIdentParticipant, BillParticipant};
use bcr_wdc_utils::convert;
use bitcoin::Amount;
use strum::Display;
use uuid::Uuid;
// ----- local modules
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

#[derive(Debug, Clone, strum::EnumDiscriminants, serde::Serialize, serde::Deserialize)]
#[strum_discriminants(derive(serde::Serialize, serde::Deserialize, Display))]
#[serde(tag = "status")]
pub enum Status {
    Pending {
        minting_pubkey: cashu::PublicKey,
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
        minting_pubkey: cashu::PublicKey,
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
        minting_pubkey: cashu::PublicKey,
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
    pub maturity_date: chrono::NaiveDate,
}

impl Quote {
    pub fn new(bill: BillInfo, minting_pubkey: cashu::PublicKey, submitted: TStamp) -> Self {
        Self {
            status: Status::Pending { minting_pubkey },
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
            Err(Error::InvalidQuoteStatus(
                self.id,
                StatusDiscriminants::Pending,
                StatusDiscriminants::from(self.status.clone()),
            ))
        }
    }

    pub fn deny(&mut self, tstamp: TStamp) -> Result<()> {
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
        let Status::Pending { minting_pubkey, .. } = self.status else {
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
            minting_pubkey,
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
        self.check_expire(tstamp);
        match self.status {
            Status::Offered {
                keyset_id,
                discounted,
                minting_pubkey,
                ..
            } => {
                self.status = Status::Accepted {
                    keyset_id,
                    discounted,
                    minting_pubkey,
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
}
