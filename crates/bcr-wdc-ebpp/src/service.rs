// ----- standard library imports
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
};
// ----- extra library imports
use async_trait::async_trait;
use bcr_wdc_webapi::signatures::{RequestToMintFromEBillDesc, SignedRequestToMintFromEBillDesc};
use bdk_wallet::bitcoin as btc;
use cdk_common::mint::MeltQuote;
use cdk_common::{
    nuts::{MeltQuoteState, MintQuoteState},
    payment::{
        Bolt11Settings, CreateIncomingPaymentResponse, Error as PaymentError, MakePaymentResponse,
        MintPayment, PaymentQuoteResponse,
    },
    {Amount, CurrencyUnit, MeltOptions},
};
use futures::Stream;
use tokio::sync::mpsc;
use uuid::Uuid;
// ----- local imports
use crate::error::Result;
use crate::payment;

// ----- end imports

type PaymentResult<T> = std::result::Result<T, PaymentError>;

#[async_trait]
pub trait OnChainWallet: Sync {
    fn generate_new_recipient(&self) -> Result<btc::Address>;
    async fn add_descriptor(&self, descriptor: &str) -> Result<btc::Address>;
    async fn balance(&self) -> Result<bdk_wallet::Balance>;
}

#[async_trait]
pub trait PaymentRepository: Sync {
    async fn load_request(&self, reqid: Uuid) -> Result<payment::Request>;
    async fn store_request(&self, req: payment::Request) -> Result<()>;
}

#[async_trait]
pub trait EBillNode: Sync {
    /// Returns a string representing the bitcoin descriptor where payment is expected
    async fn request_to_pay(&self, bill: &str, amount: Amount) -> Result<String>;
}

#[derive(Debug, Clone)]
pub struct Service<OnChainWlt, PayRepo, EBillCl> {
    pub onchain: OnChainWlt,
    pub payrepo: PayRepo,
    pub ebill: EBillCl,

    payment_notifier: Arc<Mutex<Option<mpsc::Sender<String>>>>,
}

impl<OnChainWlt, PayRepo, EBillCl> Service<OnChainWlt, PayRepo, EBillCl> {
    pub async fn new(onchain: OnChainWlt, payrepo: PayRepo, ebill: EBillCl) -> Self {
        let payment_notifier = Arc::new(Mutex::new(None));
        Self {
            onchain,
            payrepo,
            ebill,
            payment_notifier,
        }
    }
}

impl<OnChainWlt, PayRepo, EBillCl> Service<OnChainWlt, PayRepo, EBillCl>
where
    OnChainWlt: OnChainWallet,
{
    pub async fn balance(&self) -> Result<bdk_wallet::Balance> {
        self.onchain.balance().await
    }
}

#[async_trait]
impl<OnChainWlt, PayRepo, EBillCl> MintPayment for Service<OnChainWlt, PayRepo, EBillCl>
where
    OnChainWlt: OnChainWallet,
    PayRepo: PaymentRepository,
    EBillCl: EBillNode,
{
    type Err = cdk_common::payment::Error;

    async fn get_settings(&self) -> PaymentResult<serde_json::Value> {
        log::info!("get_settings");

        let settings = Bolt11Settings {
            mpp: false,
            unit: CurrencyUnit::Sat,
            invoice_description: true,
        };
        serde_json::to_value(settings).map_err(PaymentError::Serde)
    }

    async fn create_incoming_payment_request(
        &self,
        amount: Amount,
        unit: &CurrencyUnit,
        description: String,
        unix_expiry: Option<u64>,
    ) -> PaymentResult<CreateIncomingPaymentResponse> {
        log::info!(
            "create_incoming_payment_request: description: {}",
            description
        );

        let payment_t = if let Ok(request) =
            serde_json::from_str::<SignedRequestToMintFromEBillDesc>(&description)
        {
            let request = validate_ebill_request_signature(&request)?;
            let output = self.ebill.request_to_pay(&request.ebill, amount).await?;
            let address = self.onchain.add_descriptor(&output).await?;
            payment::PaymentType::EBill(address)
        } else {
            if !matches!(unit, CurrencyUnit::Sat) {
                return Err(PaymentError::UnsupportedUnit);
            };
            let address = self
                .onchain
                .generate_new_recipient()
                .map_err(PaymentError::from)?;
            payment::PaymentType::OnChain(address)
        };
        let payment = payment::Request {
            reqid: Uuid::new_v4(),
            amount,
            currency: unit.clone(),
            payment_type: payment_t,
        };
        let response = CreateIncomingPaymentResponse {
            expiry: unix_expiry,
            request_lookup_id: payment.reqid.to_string(),
            request: payment.to_string(),
        };
        Ok(response)
    }

    async fn get_payment_quote(
        &self,
        request: &str,
        _unit: &CurrencyUnit,
        _options: Option<MeltOptions>,
    ) -> PaymentResult<PaymentQuoteResponse> {
        log::info!("get_payment_quote, {}", request);

        let response = PaymentQuoteResponse {
            request_lookup_id: String::new(),
            amount: Amount::ZERO,
            fee: Amount::ZERO,
            state: MeltQuoteState::Unpaid,
        };
        Ok(response)
    }

    async fn make_payment(
        &self,
        melt_quote: MeltQuote,
        _partial_amount: Option<Amount>,
        _max_fee_amount: Option<Amount>,
    ) -> PaymentResult<MakePaymentResponse> {
        log::info!("make_payment, {}", melt_quote.request_lookup_id);

        let response = MakePaymentResponse {
            payment_lookup_id: String::new(),
            payment_proof: None,
            status: MeltQuoteState::Unpaid,
            total_spent: Amount::ZERO,
            unit: CurrencyUnit::Sat,
        };
        Ok(response)
    }

    async fn wait_any_incoming_payment(
        &self,
    ) -> PaymentResult<Pin<Box<dyn Stream<Item = String> + Send>>> {
        log::info!("wait_any_incoming_payment");

        let (sender, mut receiver) = mpsc::channel(5);
        let stream = async_stream::stream! {
                while let Some(msg) = receiver.recv().await {
                yield msg;
                }
        };
        let mut locked_sender = self.payment_notifier.lock().unwrap();
        *locked_sender = Some(sender);
        let pinned = Box::pin(stream);
        Ok(pinned)
    }

    fn is_wait_invoice_active(&self) -> bool {
        log::info!("is_wait_invoice_active");

        false
    }

    fn cancel_wait_invoice(&self) {
        log::info!("cancel_wait_invoice");
    }

    async fn check_incoming_payment_status(
        &self,
        _request_lookup_id: &str,
    ) -> PaymentResult<MintQuoteState> {
        log::info!("check_incoming_payment_status");

        let response = MintQuoteState::Unpaid;
        Ok(response)
    }

    async fn check_outgoing_payment(
        &self,
        _request_lookup_id: &str,
    ) -> PaymentResult<MakePaymentResponse> {
        log::info!("check_outgoing_payment");

        let response = MakePaymentResponse {
            payment_lookup_id: String::new(),
            payment_proof: None,
            status: MeltQuoteState::Unpaid,
            total_spent: Amount::ZERO,
            unit: CurrencyUnit::Sat,
        };
        Ok(response)
    }
}

fn validate_ebill_request_signature(
    signed: &SignedRequestToMintFromEBillDesc,
) -> Result<&RequestToMintFromEBillDesc> {
    // TODO: Implement the signature validation logic
    Ok(&signed.data)
}
