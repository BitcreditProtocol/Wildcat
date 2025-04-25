// ----- standard library imports
use std::{
    pin::Pin,
    str::FromStr,
    sync::{Arc, Mutex},
};
// ----- extra library imports
use anyhow::anyhow;
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
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
// ----- local imports
use crate::error::{Error, Result};
use crate::payment;

// ----- end imports

type PaymentResult<T> = std::result::Result<T, PaymentError>;

#[async_trait]
pub trait OnChainWallet: Sync {
    fn generate_new_recipient(&self) -> Result<btc::Address>;
    fn network(&self) -> btc::Network;
    async fn add_descriptor(&self, descriptor: &str) -> Result<btc::Address>;
    async fn balance(&self) -> Result<bdk_wallet::Balance>;
    async fn get_address_balance(&self, recipient: &btc::Address) -> Result<btc::Amount>;
    async fn estimate_fees(&self) -> Result<btc::Amount>;
    // returns (transaction_id, total_spent_fee)
    async fn send_to(
        &self,
        recipient: btc::Address,
        amount: btc::Amount,
        max_fee: btc::Amount,
    ) -> Result<(btc::Txid, btc::Amount)>;
    async fn is_confirmed(&self, tx_id: btc::Txid) -> Result<bool>;
}

#[async_trait]
pub trait PaymentRepository: Sync {
    async fn load_incoming(&self, reqid: Uuid) -> Result<payment::IncomingRequest>;
    async fn store_incoming(&self, req: payment::IncomingRequest) -> Result<()>;
    async fn update_incoming(&self, req: payment::IncomingRequest) -> Result<()>;
    async fn list_unpaid_incoming_requests(&self) -> Result<Vec<payment::IncomingRequest>>;

    async fn load_outgoing(&self, reqid: Uuid) -> Result<payment::OutgoingRequest>;
    async fn store_outgoing(&self, req: payment::OutgoingRequest) -> Result<()>;
    async fn update_outgoing(&self, req: payment::OutgoingRequest) -> Result<()>;
}

#[async_trait]
pub trait EBillNode: Sync {
    /// Returns a string representing the bitcoin descriptor where payment is expected
    async fn request_to_pay(&self, bill: &str, amount: btc::Amount) -> Result<String>;
}

#[derive(Debug, Clone)]
pub struct Service<OnChainWlt, PayRepo, EBillCl> {
    pub onchain: OnChainWlt,
    pub payrepo: PayRepo,
    pub ebill: EBillCl,

    payment_notifier: Arc<Mutex<Option<mpsc::Sender<String>>>>,
    interval: core::time::Duration,
    notif_cancel_token: Arc<Mutex<CancellationToken>>,
}

impl<OnChainWlt, PayRepo, EBillCl> Service<OnChainWlt, PayRepo, EBillCl> {
    pub async fn new(
        onchain: OnChainWlt,
        payrepo: PayRepo,
        ebill: EBillCl,
        refresh_interval: core::time::Duration,
    ) -> Self {
        let payment_notifier = Arc::new(Mutex::new(None));
        Self {
            onchain,
            payrepo,
            ebill,
            payment_notifier,
            interval: refresh_interval,
            notif_cancel_token: Arc::new(Mutex::new(CancellationToken::new())),
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
    OnChainWlt: OnChainWallet + Send + Clone + 'static,
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
            amountless: false,
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
        log::debug!(
            "create_incoming_payment_request: amount {} in {}, description: {}",
            amount,
            unit,
            description
        );

        if !matches!(unit, CurrencyUnit::Sat) {
            return Err(PaymentError::UnsupportedUnit);
        }

        let amount = btc::Amount::from_sat(amount.into());
        let parsed_description =
            serde_json::from_str::<SignedRequestToMintFromEBillDesc>(&description);
        let payment_type = match parsed_description {
            Ok(ebill_request_to_mint) => {
                let request = validate_ebill_request_signature(&ebill_request_to_mint)?;
                let output = self.ebill.request_to_pay(&request.ebill_id, amount).await?;
                let recipient = self.onchain.add_descriptor(&output).await?;
                payment::PaymentType::EBill(recipient)
            }
            Err(_) => {
                let recipient = self
                    .onchain
                    .generate_new_recipient()
                    .map_err(PaymentError::from)?;
                payment::PaymentType::OnChain(recipient)
            }
        };
        let mut uri = bip21::Uri::new(payment_type.recipient());
        uri.amount = Some(amount);
        let expiration = unix_expiry.and_then(|u| chrono::DateTime::from_timestamp(u as i64, 0));
        let request = payment::IncomingRequest {
            reqid: Uuid::new_v4(),
            payment_type,
            amount,
            expiration,
            status: MintQuoteState::Unpaid,
        };

        let reqid = request.reqid;
        let recipient = request.payment_type.recipient();
        self.payrepo.store_incoming(request).await?;
        let locked_notifier = self.payment_notifier.lock().unwrap();
        if let Some(sender) = &*locked_notifier {
            let token = CancellationToken::new();
            let cloned = token.clone();
            tokio::spawn(notify_payment(
                self.onchain.clone(),
                recipient,
                amount,
                sender.clone(),
                self.interval,
                cloned,
            ));
        }

        let response = CreateIncomingPaymentResponse {
            expiry: unix_expiry,
            request_lookup_id: reqid.to_string(),
            request: uri.to_string(),
        };
        Ok(response)
    }

    async fn get_payment_quote(
        &self,
        request: &str,
        unit: &CurrencyUnit,
        options: Option<MeltOptions>,
    ) -> PaymentResult<PaymentQuoteResponse> {
        log::debug!("get_payment_quote, {}", request);

        if options.is_some() {
            return Err(PaymentError::UnsupportedPaymentOption);
        }
        if !matches!(unit, CurrencyUnit::Sat) {
            return Err(PaymentError::UnsupportedUnit);
        }

        let uri = parse_to_bip21_uri(request, self.onchain.network())?;
        let fees_btc = self.onchain.estimate_fees().await?;
        let fee = Amount::from(fees_btc.to_sat());
        let outgoing = payment::OutgoingRequest::new(Uuid::new_v4(), uri)?;
        let response = PaymentQuoteResponse {
            request_lookup_id: outgoing.reqid.to_string(),
            amount: Amount::from(outgoing.amount.to_sat()),
            fee,
            state: outgoing.status,
        };
        self.payrepo.store_outgoing(outgoing).await?;
        Ok(response)
    }

    async fn make_payment(
        &self,
        melt_quote: MeltQuote,
        partial_amount: Option<Amount>,
        max_fee_amount: Option<Amount>,
    ) -> PaymentResult<MakePaymentResponse> {
        log::debug!("make_payment, {}", melt_quote.request_lookup_id);

        if partial_amount.is_some() {
            return Err(PaymentError::UnsupportedPaymentOption);
        }
        if max_fee_amount.is_some() {
            return Err(PaymentError::UnsupportedPaymentOption);
        }
        if !matches!(melt_quote.unit, CurrencyUnit::Sat) {
            return Err(PaymentError::UnsupportedUnit);
        }

        let reqid = Uuid::parse_str(&melt_quote.request_lookup_id)
            .map_err(|_| PaymentError::UnknownPaymentState)?;
        let outgoing = self.payrepo.load_outgoing(reqid).await;
        let mut request = match outgoing {
            Ok(request) => match request.status {
                MeltQuoteState::Paid => return Err(PaymentError::InvoiceAlreadyPaid),
                MeltQuoteState::Pending => return Err(PaymentError::InvoicePaymentPending),
                _ => request,
            },
            Err(Error::PaymentRequestNotFound(_)) => {
                let uri = parse_to_bip21_uri(&melt_quote.request, self.onchain.network())?;
                let request = payment::OutgoingRequest::new(Uuid::new_v4(), uri)?;
                self.payrepo.store_outgoing(request.clone()).await?;
                request
            }
            Err(e) => return Err(e.into()),
        };

        let quote_amount = btc::Amount::from_sat(melt_quote.amount.into());
        let reserved_fee_amount = btc::Amount::from_sat(melt_quote.fee_reserve.into());
        let uri_parse_result = parse_to_bip21_uri(&melt_quote.request, self.onchain.network());
        let recipient = match uri_parse_result {
            Ok(bip21::Uri {
                address, amount, ..
            }) => {
                if let Some(uri_amount) = amount {
                    if quote_amount != uri_amount {
                        let err_mesg = format!(
                            "bip21::Uri.amount {quote_amount} != melt_quote.amount {uri_amount}"
                        );
                        return Err(PaymentError::Amount(
                            cdk_common::amount::Error::InvalidAmount(err_mesg),
                        ));
                    }
                }
                address
            }
            Err(_) => btc::Address::from_str(&melt_quote.request)
                .map_err(|e| PaymentError::Anyhow(anyhow!(e)))?
                .require_network(self.onchain.network())
                .map_err(|e| PaymentError::Anyhow(anyhow!(e)))?,
        };

        request.status = MeltQuoteState::Pending;
        self.payrepo.update_outgoing(request.clone()).await?;
        let (tx_id, total_fee) = self
            .onchain
            .send_to(recipient, quote_amount, reserved_fee_amount)
            .await?;
        request.proof = Some(tx_id);
        let total_spent = quote_amount + total_fee;
        request.total_spent = Some(total_spent);
        let store_result = self.payrepo.update_outgoing(request.clone()).await;
        if let Err(e) = store_result {
            log::error!(
                "Error in storing proof for reqid {}, tx_id {tx_id}, e: {e}",
                request.reqid
            );
        }

        let total_spent = Amount::from(total_spent.to_sat());
        let response = MakePaymentResponse {
            payment_lookup_id: tx_id.to_string(),
            payment_proof: Some(tx_id.to_string()),
            status: MeltQuoteState::Pending,
            total_spent,
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
        let locked_sender = self.payment_notifier.lock().unwrap();
        if let Some(sender) = &*locked_sender {
            !sender.is_closed()
        } else {
            false
        }
    }

    fn cancel_wait_invoice(&self) {
        log::info!("cancel_wait_invoice");
        *self.payment_notifier.lock().unwrap() = None;
        let mut locked = self.notif_cancel_token.lock().unwrap();
        locked.cancel();
        *locked = CancellationToken::new();
    }

    async fn check_incoming_payment_status(
        &self,
        request_lookup_id: &str,
    ) -> PaymentResult<MintQuoteState> {
        log::info!("check_incoming_payment_status");

        let reqid =
            Uuid::parse_str(request_lookup_id).map_err(|_| PaymentError::UnknownPaymentState)?;
        let mut request = self.payrepo.load_incoming(reqid).await?;
        let mut response = request.status;
        if request.status == MintQuoteState::Unpaid {
            request.status = check_incoming_payment(
                &request.payment_type.recipient(),
                request.amount,
                &self.onchain,
            )
            .await?;
            response = request.status;
            self.payrepo.update_incoming(request).await?;
        }
        Ok(response)
    }

    async fn check_outgoing_payment(
        &self,
        request_lookup_id: &str,
    ) -> PaymentResult<MakePaymentResponse> {
        log::warn!("check_outgoing_payment {}", request_lookup_id);

        let reqid =
            Uuid::parse_str(request_lookup_id).map_err(|_| PaymentError::UnknownPaymentState)?;
        let mut request = self.payrepo.load_outgoing(reqid).await?;

        let total_spent = Amount::from(request.total_spent.unwrap_or(request.amount).to_sat());
        let response = MakePaymentResponse {
            payment_lookup_id: request_lookup_id.to_string(),
            payment_proof: request.proof.map(|txid| txid.to_string()),
            unit: CurrencyUnit::Sat,
            status: request.status,
            total_spent,
        };
        if matches!(request.status, MeltQuoteState::Paid) {
            return Ok(response);
        }

        let new_state = check_outgoing_payment(request.proof, &self.onchain).await?;
        request.status = new_state;
        self.payrepo.update_outgoing(request).await?;
        Ok(response)
    }
}

fn validate_ebill_request_signature(
    signed: &SignedRequestToMintFromEBillDesc,
) -> Result<&RequestToMintFromEBillDesc> {
    // TODO: Implement the signature validation logic
    Ok(&signed.data)
}

async fn notify_payment<OnChain>(
    onchain: OnChain,
    recipient: btc::Address,
    expected: btc::Amount,
    sender: mpsc::Sender<String>,
    pause: core::time::Duration,
    token: CancellationToken,
) where
    OnChain: OnChainWallet,
{
    loop {
        tokio::select! {
            _ = token.cancelled() => {
                log::info!("wallet update loop stopping");
                break;
            }
            _ = tokio::time::sleep(pause) => {
                log::debug!("wallet update loop waking up");
            }
        }

        if sender.is_closed() {
            log::warn!("validate_ebill_request_signature for recipient {recipient}, channel closed, exiting");
            return;
        }
        let state_res = check_incoming_payment(&recipient, expected, &onchain).await;
        if let Err(e) = state_res {
            log::error!("error in checking payment for recipient {recipient}, error: {e}");
            continue;
        }
    }
}

async fn check_incoming_payment<OnChain>(
    recipient: &btc::Address,
    expected: btc::Amount,
    onchain: &OnChain,
) -> Result<cdk_common::MintQuoteState>
where
    OnChain: OnChainWallet,
{
    let amount = onchain.get_address_balance(recipient).await?;
    if amount >= expected {
        Ok(cdk_common::MintQuoteState::Paid)
    } else {
        Ok(cdk_common::MintQuoteState::Unpaid)
    }
}

async fn check_outgoing_payment<OnChain>(
    tx_id: Option<btc::Txid>,
    onchain: &OnChain,
) -> Result<cdk_common::MeltQuoteState>
where
    OnChain: OnChainWallet,
{
    if let Some(tx_id) = tx_id {
        if onchain.is_confirmed(tx_id).await? {
            Ok(cdk_common::MeltQuoteState::Paid)
        } else {
            Ok(cdk_common::MeltQuoteState::Pending)
        }
    } else {
        Ok(cdk_common::MeltQuoteState::Unpaid)
    }
}

fn parse_to_bip21_uri(input: &str, network: btc::Network) -> Result<bip21::Uri> {
    bip21::Uri::from_str(input)
        .map_err(|e| Error::Bip21Parse(anyhow!(e)))?
        .require_network(network)
        .map_err(|e| Error::Bip21Parse(anyhow!(e)))
}
