// ----- standard library imports
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
};
// ----- extra library imports
use async_trait::async_trait;
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

// ----- end imports

type PaymentResult<T> = std::result::Result<T, PaymentError>;

#[async_trait]
pub trait OnChainWallet: Sync {
    async fn new_payment_request(&self, amount: bitcoin::Amount) -> Result<bip21::Uri>;
    async fn balance(&self) -> Result<bdk_wallet::Balance>;
}

#[derive(Debug, Clone)]
pub struct Service<OnChainWlt> {
    pub onchain: OnChainWlt,
    payment_notifier: Arc<Mutex<Option<mpsc::Sender<String>>>>,
}

impl<OnChainWlt> Service<OnChainWlt> {
    pub async fn new(onchain: OnChainWlt) -> Self {
        let payment_notifier = Arc::new(Mutex::new(None));
        Self {
            onchain,
            payment_notifier,
        }
    }
}

impl<OnChainWlt> Service<OnChainWlt>
where
    OnChainWlt: OnChainWallet,
{
    pub async fn balance(&self) -> Result<bdk_wallet::Balance> {
        self.onchain.balance().await
    }
}

#[async_trait]
impl<OnChainWlt> MintPayment for Service<OnChainWlt>
where
    OnChainWlt: OnChainWallet,
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
        _unix_expiry: Option<u64>,
    ) -> PaymentResult<CreateIncomingPaymentResponse> {
        log::info!(
            "create_incoming_payment_request: description: {}",
            description
        );
        let amount = match unit {
            CurrencyUnit::Sat => bitcoin::Amount::from_sat(*amount.as_ref()),
            CurrencyUnit::Msat => bitcoin::Amount::from_sat(*amount.as_ref() * 1000_u64),
            _ => return Err(PaymentError::UnsupportedUnit),
        };
        let qid = Uuid::new_v4();
        let uri = self
            .onchain
            .new_payment_request(amount)
            .await
            .map_err(PaymentError::from)?;
        let response = CreateIncomingPaymentResponse {
            expiry: None,
            request_lookup_id: qid.to_string(),
            request: uri.to_string(),
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
