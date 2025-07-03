// ----- standard library imports
// ----- extra library imports
use bcr_wdc_webapi::{
    bill::{
        BillCombinedBitcoinKey, BillId, BillsResponse, BitcreditBill, Endorsement,
        RequestToPayBitcreditBillPayload,
    },
    identity::{Identity, NewIdentityPayload, SeedPhrase},
    quotes::{BillInfo, SharedBill},
};
use reqwest::header;
pub use reqwest::Url;
use thiserror::Error;
// ----- local imports
// ----- end imports

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Error)]
pub enum Error {
    #[error("resource not found {0}")]
    ResourceNotFound(String),
    #[error("invalid request")]
    InvalidRequest,
    #[error("invalid content type")]
    InvalidContentType,
    #[error("invalid bill id")]
    InvalidBillId,
    #[error("internal error {0}")]
    Reqwest(#[from] reqwest::Error),
}

#[derive(Debug, Clone)]
pub struct EbillClient {
    cl: reqwest::Client,
    base: reqwest::Url,
}

impl EbillClient {
    pub fn new(base: reqwest::Url) -> Self {
        Self {
            cl: reqwest::Client::new(),
            base,
        }
    }

    pub async fn validate_and_decrypt_shared_bill(
        &self,
        shared_bill: &SharedBill,
    ) -> Result<BillInfo> {
        let url = self
            .base
            .join("/v1/bill/validate_and_decrypt_shared_bill")
            .expect("validate and decrypt shared bill relative path");
        let res = self
            .cl
            .post(url)
            .json(shared_bill)
            .send()
            .await?
            .error_for_status()?;
        if res.status() == reqwest::StatusCode::BAD_REQUEST {
            return Err(Error::InvalidRequest);
        }
        let bill_info = res.json::<BillInfo>().await?;
        Ok(bill_info)
    }

    pub async fn backup_seed_phrase(&self) -> Result<SeedPhrase> {
        let url = self
            .base
            .join("/v1/identity/seed/backup")
            .expect("seed phrase relative path");
        let res = self.cl.get(url).send().await?;
        let seed_phrase = res.json::<SeedPhrase>().await?;
        Ok(seed_phrase)
    }

    pub async fn restore_from_seed_phrase(&self, seed_phrase: &SeedPhrase) -> Result<()> {
        let url = self
            .base
            .join("/v1/identity/seed/recover")
            .expect("seed phrase backup relative path");
        let res = self.cl.put(url).json(seed_phrase).send().await?;
        if res.status() == reqwest::StatusCode::BAD_REQUEST {
            return Err(Error::InvalidRequest);
        }
        res.error_for_status()?;
        Ok(())
    }

    pub async fn get_identity(&self) -> Result<Identity> {
        let url = self
            .base
            .join("/v1/identity/detail")
            .expect("identity detail relative path");
        let res = self.cl.get(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound("identity".into()));
        }
        let identity = res.json::<Identity>().await?;
        Ok(identity)
    }

    pub async fn create_identity(&self, payload: &NewIdentityPayload) -> Result<()> {
        let url = self
            .base
            .join("/v1/identity/create")
            .expect("create identity relative path");
        let res = self.cl.post(url).json(payload).send().await?;
        if res.status() == reqwest::StatusCode::BAD_REQUEST {
            return Err(Error::InvalidRequest);
        }
        res.error_for_status()?;
        Ok(())
    }

    pub async fn get_bills(&self) -> Result<Vec<BitcreditBill>> {
        let url = self
            .base
            .join("/v1/bill/list")
            .expect("bill list relative path");
        let res = self.cl.get(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound("identity".into()));
        }
        let bills = res.json::<BillsResponse<BitcreditBill>>().await?;
        Ok(bills.bills)
    }

    pub async fn get_bill(&self, bill_id: &BillId) -> Result<BitcreditBill> {
        let url = self
            .base
            .join(&format!("/v1/bill/detail/{bill_id}"))
            .expect("bill detail relative path");
        let res = self.cl.get(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(bill_id.to_string()));
        }
        let bill = res.json::<BitcreditBill>().await?;
        Ok(bill)
    }

    pub async fn get_bill_endorsements(&self, bill_id: &BillId) -> Result<Vec<Endorsement>> {
        let url = self
            .base
            .join(&format!("/v1/bill/endorsements/{bill_id}"))
            .expect("bill detail relative path");
        let res = self.cl.get(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(bill_id.to_string()));
        }
        let endorsements = res.json::<Vec<Endorsement>>().await?;
        Ok(endorsements)
    }

    /// Returns the content type and the bytes of the file
    pub async fn get_bill_attachment(
        &self,
        bill_id: &BillId,
        file_name: &str,
    ) -> Result<(String, Vec<u8>)> {
        let url = self
            .base
            .join(&format!("/v1/bill/attachment/{bill_id}/{file_name}"))
            .expect("bill attachment relative path");
        let res = self.cl.get(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(format!("{bill_id} - {file_name}",)));
        }
        let content_type: String = match res.headers().get(header::CONTENT_TYPE).map(|h| h.to_str())
        {
            Some(Ok(content_type)) => content_type.to_owned(),
            _ => return Err(Error::InvalidContentType),
        };
        let bytes = res.bytes().await?;
        Ok((content_type, bytes.to_vec()))
    }

    pub async fn get_bitcoin_private_descriptor_for_bill(
        &self,
        bill_id: &BillId,
    ) -> Result<BillCombinedBitcoinKey> {
        let url = self
            .base
            .join(&format!("/v1/bill/bitcoin_key/{bill_id}"))
            .expect("bill bitcoin key relative path");
        let res = self.cl.get(url).send().await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(bill_id.to_string()));
        }
        let btc_key = res.json::<BillCombinedBitcoinKey>().await?;
        Ok(btc_key)
    }

    pub async fn request_to_pay_bill(
        &self,
        payload: &RequestToPayBitcreditBillPayload,
    ) -> Result<()> {
        let url = self
            .base
            .join("/v1/bill/request_to_pay")
            .expect("req to pay bill relative path");
        let res = self.cl.put(url).json(payload).send().await?;
        if res.status() == reqwest::StatusCode::BAD_REQUEST {
            return Err(Error::InvalidRequest);
        }
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::ResourceNotFound(payload.bill_id.to_string()));
        }
        res.error_for_status()?;
        Ok(())
    }
}
