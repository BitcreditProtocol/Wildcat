// ----- standard library imports
use std::{
    collections::HashMap,
    pin::Pin,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};
// ----- extra library imports
use async_trait::async_trait;
use bcr_common::{
    core::{
        signature::{deserialize_borsh_msg, schnorr_verify_b64},
        BillId,
    },
    wire::signatures as wire_signatures,
};
use bcr_wdc_webapi::exchange as web_exchange;
use bdk_wallet::bitcoin as btc;
use cdk_common::{
    nuts::{MeltQuoteState, MintQuoteState},
    payment::{
        Bolt11Settings, CreateIncomingPaymentResponse, Error as PaymentError, Event,
        IncomingPaymentOptions, MakePaymentResponse, MintPayment, OutgoingPaymentOptions,
        PaymentIdentifier, PaymentQuoteResponse, WaitPaymentResponse,
    },
    CurrencyUnit,
};
use futures::Stream;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    payment,
    persistence::PaymentRepository,
    TStamp,
};

// ----- end imports

type PaymentResult<T> = std::result::Result<T, PaymentError>;

#[async_trait]
pub trait OnChainWallet: Send + Sync {
    fn network(&self) -> btc::Network;
    async fn add_descriptor(&self, descriptor: &str) -> Result<btc::Address>;
    async fn balance(&self) -> Result<bdk_wallet::Balance>;
    async fn get_address_balance(&self, recipient: &btc::Address) -> Result<btc::Amount>;
    async fn estimate_fees(&self) -> Result<btc::Amount>;
    // returns (transaction_id, total_spent_fee)
    async fn sweep_address_to(
        &self,
        address: btc::Address,
        recipient: btc::Address,
        max_fee: btc::Amount,
    ) -> Result<btc::Txid>;
    async fn is_confirmed(&self, tx_id: btc::Txid) -> Result<bool>;
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait EBillNode: Send + Sync {
    /// Returns a string representing the bitcoin descriptor where payment is expected
    async fn request_to_pay(
        &self,
        bill: &BillId,
        amount: btc::Amount,
        deadline: TStamp,
    ) -> Result<String>;
}

#[derive(Clone)]
pub struct Service {
    onchain: Arc<dyn OnChainWallet>,
    payrepo: Arc<dyn PaymentRepository>,
    ebill: Arc<dyn EBillNode>,
    treasury_pubkey: btc::secp256k1::XOnlyPublicKey,

    payment_notifier: Arc<Mutex<Option<mpsc::Sender<Event>>>>,
    interval: Duration,
    notif_cancel_token: Arc<Mutex<CancellationToken>>,
    dev_payments: Arc<Mutex<HashMap<PaymentIdentifier, cashu::Amount>>>,
}

impl Service {
    pub async fn new(
        onchain: Arc<dyn OnChainWallet>,
        payrepo: Arc<dyn PaymentRepository>,
        ebill: Arc<dyn EBillNode>,
        refresh_interval: Duration,
        treasury_pubkey: btc::secp256k1::XOnlyPublicKey,
    ) -> Self {
        let payment_notifier = Arc::new(Mutex::new(None));
        Self {
            onchain,
            payrepo,
            ebill,
            treasury_pubkey,
            payment_notifier,
            interval: refresh_interval,
            notif_cancel_token: Arc::new(Mutex::new(CancellationToken::new())),
            dev_payments: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Service {
    pub fn network(&self) -> btc::Network {
        self.onchain.network()
    }

    pub async fn balance(&self) -> Result<bdk_wallet::Balance> {
        self.onchain.balance().await
    }
}

#[async_trait]
impl MintPayment for Service {
    type Err = cdk_common::payment::Error;

    async fn get_settings(&self) -> PaymentResult<serde_json::Value> {
        let _span = tracing::debug_span!("get_settings");

        let settings = Bolt11Settings {
            mpp: false,
            unit: CurrencyUnit::Sat,
            invoice_description: true,
            amountless: false,
            bolt12: false,
        };
        serde_json::to_value(settings).map_err(PaymentError::Serde)
    }

    async fn create_incoming_payment_request(
        &self,
        unit: &CurrencyUnit,
        options: IncomingPaymentOptions,
    ) -> PaymentResult<CreateIncomingPaymentResponse> {
        tracing::trace!(
            unit = ?unit,
            "create_incoming_payment_request"
        );

        if !matches!(unit, CurrencyUnit::Sat) {
            return Err(PaymentError::UnsupportedUnit);
        }
        let IncomingPaymentOptions::Bolt11(options) = options else {
            return Err(PaymentError::UnsupportedPaymentOption);
        };
        let amount = btc::Amount::from_sat(options.amount.into());
        let parsed_description = ParsedDescription::parse(&options.description.unwrap_or_default());
        let payment_type = match parsed_description {
            Ok(ParsedDescription::ForeignECash(request)) => {
                tracing::debug!("Parsed foreign ecash request",);
                let payload: web_exchange::RequestToMintFromForeigneCashPayload =
                    deserialize_borsh_msg(&request.payload).map_err(Error::from)?;
                schnorr_verify_b64(&request.payload, &request.signature, &self.treasury_pubkey)
                    .map_err(Error::from)?;
                let foreign = self.payrepo.check_foreign_nonce(&payload.nonce).await?;
                if foreign.is_some() {
                    return Err(PaymentError::InvoiceAlreadyPaid);
                }
                let reqid = PaymentIdentifier::CustomId(Uuid::new_v4().to_string());
                let response = CreateIncomingPaymentResponse {
                    request_lookup_id: reqid.clone(),
                    request: String::new(),
                    expiry: None,
                };
                let foreign = payment::ForeignPayment {
                    reqid,
                    nonce: payload.nonce.clone(),
                    amount,
                };
                self.payrepo.store_foreign(foreign).await?;
                return Ok(response);
            }
            Ok(ParsedDescription::EbillRequestToPay(request)) => {
                tracing::trace!("Parsed EBill request",);
                schnorr_verify_b64(&request.content, &request.signature, &self.treasury_pubkey)
                    .map_err(Error::from)?;
                let message: wire_signatures::RequestToMintFromEBillDesc =
                    deserialize_borsh_msg(&request.content).map_err(Error::from)?;
                let output = self
                    .ebill
                    .request_to_pay(&message.ebill_id, amount, message.deadline)
                    .await?;
                let recipient = self.onchain.add_descriptor(&output).await?;
                let sweep = btc::Address::from_str(&message.sweeping_address)
                    .map_err(|_| PaymentError::UnsupportedPaymentOption)?
                    .require_network(self.onchain.network())
                    .map_err(|_| PaymentError::UnsupportedPaymentOption)?;
                payment::PaymentType::EBill { recipient, sweep }
            }
            Ok(ParsedDescription::ClowderOnchain(uuid)) => {
                payment::PaymentType::ClowderOnchain(uuid)
            }
            Ok(ParsedDescription::Dev) => {
                let idx: u32 = rand::random();
                let pid = PaymentIdentifier::Label(format!("dev{idx}"));
                self.dev_payments
                    .lock()
                    .unwrap()
                    .insert(pid.clone(), options.amount);
                return Ok(CreateIncomingPaymentResponse {
                    request_lookup_id: pid,
                    request: String::new(),
                    expiry: None,
                });
            }
            Err(e) => {
                tracing::error!("Empty or unrecognized description field");
                return Err(e);
            }
        };
        let expiration = options
            .unix_expiry
            .and_then(|u| chrono::DateTime::from_timestamp(u as i64, 0));
        let request = payment::IncomingRequest {
            reqid: PaymentIdentifier::CustomId(Uuid::new_v4().to_string()),
            payment_type,
            amount,
            expiration,
            status: MintQuoteState::Unpaid,
            sweep_tx: None,
        };
        let reqid = request.reqid.clone();
        let recipient = request.payment_type.recipient();
        let request_str = recipient
            .as_ref()
            .map(|r| {
                let mut uri = bip21::Uri::new(r.clone());
                uri.amount = Some(amount);
                uri.to_string()
            })
            .unwrap_or_default();
        self.payrepo.store_incoming(request).await?;
        if let Some(recipient) = recipient {
            let locked_notifier = self.payment_notifier.lock().unwrap();
            if let Some(sender) = &*locked_notifier {
                let cloned = self.notif_cancel_token.lock().unwrap().clone();
                tokio::spawn(notify_payment(
                    self.onchain.clone(),
                    recipient,
                    amount,
                    sender.clone(),
                    self.interval,
                    cloned,
                ));
            }
        }
        let response = CreateIncomingPaymentResponse {
            expiry: options.unix_expiry,
            request_lookup_id: reqid,
            request: request_str,
        };
        Ok(response)
    }

    async fn get_payment_quote(
        &self,
        unit: &CurrencyUnit,
        options: OutgoingPaymentOptions,
    ) -> PaymentResult<PaymentQuoteResponse> {
        let _span = tracing::debug_span!("get_payment_quote");

        if !matches!(unit, CurrencyUnit::Sat) {
            return Err(PaymentError::UnsupportedUnit);
        }
        let OutgoingPaymentOptions::Bolt11(options) = options else {
            return Err(PaymentError::UnsupportedPaymentOption);
        };
        let description = options.bolt11.description().to_string();
        let request: wire_signatures::SignedRequestToMeltDesc =
            serde_json::from_str(&description).map_err(PaymentError::Serde)?;
        schnorr_verify_b64(&request.content, &request.signature, &self.treasury_pubkey)
            .map_err(|_| PaymentError::UnsupportedPaymentOption)?;
        let content: wire_signatures::RequestToMeltDesc =
            deserialize_borsh_msg(&request.content)
                .map_err(|_| PaymentError::UnsupportedPaymentOption)?;
        let reqid = PaymentIdentifier::CustomId(content.qid.to_string());
        let outgoing = payment::OutgoingRequest::new(reqid, content.amount);
        let response = PaymentQuoteResponse {
            request_lookup_id: Some(outgoing.reqid.clone()),
            amount: content.amount,
            unit: CurrencyUnit::Sat,
            fee: cashu::Amount::ZERO,
            state: MeltQuoteState::Paid,
        };
        self.payrepo.store_outgoing(outgoing).await?;
        Ok(response)
    }

    async fn make_payment(
        &self,
        unit: &CurrencyUnit,
        options: OutgoingPaymentOptions,
    ) -> PaymentResult<MakePaymentResponse> {
        let _span = tracing::debug_span!("make_payment");

        if !matches!(unit, CurrencyUnit::Sat) {
            return Err(PaymentError::UnsupportedUnit);
        }
        let OutgoingPaymentOptions::Bolt11(options) = options else {
            return Err(PaymentError::UnsupportedPaymentOption);
        };
        let description = options.bolt11.description().to_string();
        let request: wire_signatures::SignedRequestToMeltDesc =
            serde_json::from_str(&description).map_err(PaymentError::Serde)?;
        schnorr_verify_b64(&request.content, &request.signature, &self.treasury_pubkey)
            .map_err(|_| PaymentError::UnsupportedPaymentOption)?;
        let content: wire_signatures::RequestToMeltDesc =
            deserialize_borsh_msg(&request.content)
                .map_err(|_| PaymentError::UnsupportedPaymentOption)?;
        let invoice_amount =
            cashu::Amount::from(options.bolt11.amount_milli_satoshis().unwrap_or_default() / 1000);
        if invoice_amount != content.amount {
            return Err(PaymentError::Amount(cashu::amount::Error::InvalidAmount(
                format!(
                    "Invoice amount {invoice_amount} does not match requested amount {}",
                    content.amount
                ),
            )));
        }
        let reqid = PaymentIdentifier::CustomId(content.qid.to_string());
        let mut outgoing = self.payrepo.load_outgoing(&reqid).await?;
        if matches!(outgoing.state, MeltQuoteState::Paid) {
            return Err(PaymentError::InvoiceAlreadyPaid);
        }
        let response = MakePaymentResponse {
            payment_lookup_id: outgoing.reqid.clone(),
            payment_proof: None,
            status: MeltQuoteState::Paid,
            total_spent: outgoing.amount,
            unit: CurrencyUnit::Sat,
        };
        outgoing.state = MeltQuoteState::Paid;
        self.payrepo.update_outgoing(outgoing).await?;
        Ok(response)
    }

    async fn wait_payment_event(&self) -> PaymentResult<Pin<Box<dyn Stream<Item = Event> + Send>>> {
        let _span = tracing::debug_span!("wait_any_incoming_payment");

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
        let _span = tracing::debug_span!("is_wait_invoice_active");
        let locked_sender = self.payment_notifier.lock().unwrap();
        if let Some(sender) = &*locked_sender {
            !sender.is_closed()
        } else {
            false
        }
    }

    fn cancel_wait_invoice(&self) {
        let _span = tracing::debug_span!("cancel_wait_invoice");
        *self.payment_notifier.lock().unwrap() = None;
        let mut locked = self.notif_cancel_token.lock().unwrap();
        locked.cancel();
        *locked = CancellationToken::new();
    }

    async fn check_incoming_payment_status(
        &self,
        payment_identifier: &PaymentIdentifier,
    ) -> PaymentResult<Vec<WaitPaymentResponse>> {
        let _span = tracing::debug_span!("check_incoming_payment_status");

        // Dev payment
        if let Some(amount) = self.dev_payments.lock().unwrap().get(payment_identifier) {
            return Ok(vec![WaitPaymentResponse {
                payment_identifier: payment_identifier.clone(),
                payment_amount: *amount,
                unit: CurrencyUnit::Sat,
                payment_id: payment_identifier.to_string(),
            }]);
        }

        // Foreign eCash payment
        let mut response = Vec::new();
        let foreign = self.payrepo.check_foreign_reqid(payment_identifier).await?;
        if let Some(foreign) = foreign {
            let payment_id = foreign.reqid.to_string();
            response.push(WaitPaymentResponse {
                payment_identifier: foreign.reqid,
                payment_amount: cashu::Amount::from(foreign.amount.to_sat()),
                unit: CurrencyUnit::Sat,
                payment_id,
            });
            return Ok(response);
        }

        // E-Bill payment
        let mut request = self.payrepo.load_incoming(payment_identifier).await?;
        match (request.status, request.payment_type.clone()) {
            (MintQuoteState::Unpaid, payment::PaymentType::ClowderOnchain(_)) => {
                request.status = MintQuoteState::Paid;
                self.payrepo.update_incoming(request.clone()).await?;
                Ok(vec![WaitPaymentResponse {
                    payment_identifier: payment_identifier.clone(),
                    payment_amount: cashu::Amount::from(request.amount.to_sat()),
                    unit: CurrencyUnit::Sat,
                    payment_id: payment_identifier.to_string(),
                }])
            }
            (MintQuoteState::Unpaid, payment::PaymentType::EBill { recipient, sweep }) => {
                let state =
                    check_incoming_payment(&recipient, request.amount, self.onchain.as_ref())
                        .await?;
                match state {
                    MintQuoteState::Unpaid => Ok(vec![]),
                    MintQuoteState::Paid => {
                        request.status = MintQuoteState::Paid;
                        let max_fees = self.onchain.estimate_fees().await?;
                        let txid = self
                            .onchain
                            .sweep_address_to(recipient, sweep, max_fees)
                            .await?;
                        request.sweep_tx = Some(txid);
                        self.payrepo.update_incoming(request).await?;
                        Ok(vec![])
                    }
                    _ => Err(PaymentError::UnknownPaymentState),
                }
            }
            (MintQuoteState::Paid, payment::PaymentType::ClowderOnchain(_)) => {
                request.status = MintQuoteState::Issued;
                self.payrepo.update_incoming(request.clone()).await?;
                Ok(vec![WaitPaymentResponse {
                    payment_identifier: payment_identifier.clone(),
                    payment_amount: cashu::Amount::from(request.amount.to_sat()),
                    unit: CurrencyUnit::Sat,
                    payment_id: payment_identifier.to_string(),
                }])
            }
            (MintQuoteState::Paid, payment::PaymentType::EBill { .. }) => match request.sweep_tx {
                Some(txid) => {
                    if self.onchain.is_confirmed(txid).await? {
                        request.status = MintQuoteState::Issued;
                        self.payrepo.update_incoming(request.clone()).await?;
                        Ok(vec![WaitPaymentResponse {
                            payment_identifier: payment_identifier.clone(),
                            payment_amount: cashu::Amount::from(request.amount.to_sat()),
                            unit: CurrencyUnit::Sat,
                            payment_id: payment_identifier.to_string(),
                        }])
                    } else {
                        Ok(vec![])
                    }
                }
                None => {
                    tracing::error!("No sweep transaction found for paid E-Bill payment");
                    return Err(PaymentError::UnknownPaymentState);
                }
            },
            (MintQuoteState::Issued, _) => Ok(vec![WaitPaymentResponse {
                payment_identifier: payment_identifier.clone(),
                payment_amount: cashu::Amount::from(request.amount.to_sat()),
                unit: CurrencyUnit::Sat,
                payment_id: payment_identifier.to_string(),
            }]),
        }
    }

    async fn check_outgoing_payment(
        &self,
        payment_identifier: &PaymentIdentifier,
    ) -> PaymentResult<MakePaymentResponse> {
        let _span = tracing::debug_span!("check_outgoing_payment", payment_identifier = %payment_identifier);

        let request = self.payrepo.load_outgoing(payment_identifier).await?;
        let total_spent = request.amount;
        let response = MakePaymentResponse {
            payment_lookup_id: payment_identifier.clone(),
            payment_proof: None,
            unit: CurrencyUnit::Sat,
            status: MeltQuoteState::Paid,
            total_spent,
        };
        Ok(response)
    }
}

async fn notify_payment(
    onchain: Arc<dyn OnChainWallet>,
    recipient: btc::Address,
    expected: btc::Amount,
    sender: mpsc::Sender<Event>,
    pause: Duration,
    token: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = token.cancelled() => {
                tracing::info!("wallet update loop stopping");
                break;
            }
            _ = tokio::time::sleep(pause) => {
                tracing::debug!("wallet update loop waking up");
            }
        }

        if sender.is_closed() {
            tracing::warn!("validate_ebill_request_signature for recipient {recipient}, channel closed, exiting");
            return;
        }
        let state_res = check_incoming_payment(&recipient, expected, onchain.as_ref()).await;
        if let Err(e) = state_res {
            tracing::error!("error in checking payment for recipient {recipient}, error: {e}");
            continue;
        }
    }
}

async fn check_incoming_payment(
    recipient: &btc::Address,
    expected: btc::Amount,
    onchain: &dyn OnChainWallet,
) -> Result<cdk_common::MintQuoteState> {
    let amount = onchain.get_address_balance(recipient).await?;
    if amount >= expected {
        Ok(cdk_common::MintQuoteState::Paid)
    } else {
        Ok(cdk_common::MintQuoteState::Unpaid)
    }
}

enum ParsedDescription {
    Dev,
    EbillRequestToPay(wire_signatures::SignedRequestToMintFromEBillDesc),
    ForeignECash(web_exchange::RequestToMintFromForeigneCash),
    ClowderOnchain(Uuid),
}
impl ParsedDescription {
    fn parse(input: &str) -> PaymentResult<Self> {
        if let Ok(ebill_request) =
            serde_json::from_str::<wire_signatures::SignedRequestToMintFromEBillDesc>(input)
        {
            Ok(Self::EbillRequestToPay(ebill_request))
        } else if let Ok(foreign_ecash_request) =
            serde_json::from_str::<web_exchange::RequestToMintFromForeigneCash>(input)
        {
            Ok(Self::ForeignECash(foreign_ecash_request))
        } else if let Some(uuid_str) = input.strip_prefix("clowder:") {
            if let Ok(uuid) = Uuid::parse_str(uuid_str) {
                Ok(Self::ClowderOnchain(uuid))
            } else {
                Err(PaymentError::UnsupportedPaymentOption)
            }
        } else if input == "it's me, Mario" {
            Ok(Self::Dev)
        } else {
            Err(PaymentError::UnsupportedPaymentOption)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::MockPaymentRepository;
    use bcr_common::{
        core::signature::serialize_n_schnorr_sign_borsh_msg, core_tests::generate_random_keypair,
    };
    use bdk_wallet::bitcoin::hashes::{sha256, Hash};
    use cashu::lightning_invoice as ln;
    use cdk_common::payment::{Bolt11IncomingPaymentOptions, Bolt11OutgoingPaymentOptions};
    use mockall::predicate::*;

    fn generate_fake_bolt11(
        description: wire_signatures::SignedRequestToMeltDesc,
        amount: cashu::Amount,
    ) -> ln::Bolt11Invoice {
        let payment_hash = sha256::Hash::from_slice(&[0; 32][..]).unwrap();
        let payment_secret = ln::PaymentSecret([42u8; 32]);
        let sk = secp256k1::SecretKey::new(&mut rand::thread_rng());
        let description = serde_json::to_string(&description).unwrap();
        ln::InvoiceBuilder::new(ln::Currency::Bitcoin)
            .description(description)
            .payment_hash(payment_hash)
            .payment_secret(payment_secret)
            .current_timestamp()
            .amount_milli_satoshis(u64::from(amount) * 1000)
            .min_final_cltv_expiry_delta(144)
            .build_signed(|hash| secp256k1::global::SECP256K1.sign_ecdsa_recoverable(hash, &sk))
            .unwrap()
    }

    mockall::mock! {
        OnChainWallet{}
        #[async_trait]
        impl OnChainWallet for OnChainWallet {
            fn network(&self) -> btc::Network;
            async fn add_descriptor(&self, descriptor: &str) -> Result<btc::Address>;
            async fn balance(&self) -> Result<bdk_wallet::Balance>;
            async fn get_address_balance(&self, recipient: &btc::Address) -> Result<btc::Amount>;
            async fn estimate_fees(&self) -> Result<btc::Amount>;
            async fn sweep_address_to(
                &self,
                address: btc::Address,
                recipient: btc::Address,
                max_fee: btc::Amount,
            ) -> Result<btc::Txid>;
            async fn is_confirmed(&self, tx_id: btc::Txid) -> Result<bool>;
        }

        impl std::clone::Clone for OnChainWallet {
            fn clone(&self) -> Self;
        }

    }

    #[tokio::test]
    async fn create_incoming_payment_request_wrongunit() {
        let onchain = Arc::new(MockOnChainWallet::new());
        let payrepo = Arc::new(MockPaymentRepository::new());
        let ebill = Arc::new(MockEBillNode::new());
        let interval = Duration::from_secs(1);
        let tkp = generate_random_keypair();
        let srvc = Service::new(
            onchain,
            payrepo,
            ebill,
            interval,
            tkp.public_key().x_only_public_key().0,
        )
        .await;
        let result = srvc
            .create_incoming_payment_request(
                &CurrencyUnit::Usd,
                IncomingPaymentOptions::Bolt11(Bolt11IncomingPaymentOptions {
                    description: None,
                    amount: cashu::Amount::ZERO,
                    unix_expiry: None,
                }),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn create_incoming_payment_request_no_description() {
        let onchain = MockOnChainWallet::new();
        let tkp = generate_random_keypair();
        let mut payrepo = MockPaymentRepository::new();
        payrepo.expect_store_incoming().returning(|_| Ok(()));
        let ebill = Arc::new(MockEBillNode::new());
        let interval = Duration::from_secs(1);
        let srvc = Service::new(
            Arc::new(onchain),
            Arc::new(payrepo),
            ebill,
            interval,
            tkp.x_only_public_key().0,
        )
        .await;
        let result = srvc
            .create_incoming_payment_request(
                &CurrencyUnit::Sat,
                IncomingPaymentOptions::Bolt11(Bolt11IncomingPaymentOptions {
                    description: None,
                    amount: cashu::Amount::from(100),
                    unix_expiry: None,
                }),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn make_payment_wrongunit() {
        let onchain = Arc::new(MockOnChainWallet::new());
        let payrepo = Arc::new(MockPaymentRepository::new());
        let ebill = Arc::new(MockEBillNode::new());
        let interval = Duration::from_secs(1);
        let tkp = generate_random_keypair();
        let srvc = Service::new(onchain, payrepo, ebill, interval, tkp.x_only_public_key().0).await;
        let description = wire_signatures::RequestToMeltDesc {
            qid: Uuid::new_v4(),
            amount: cashu::Amount::ZERO,
        };
        let (content, payload) = serialize_n_schnorr_sign_borsh_msg(&description, &tkp).unwrap();
        let signed_desc = wire_signatures::SignedRequestToMeltDesc {
            content,
            signature: payload,
        };
        let result = srvc
            .make_payment(
                &cashu::CurrencyUnit::Usd,
                OutgoingPaymentOptions::Bolt11(Box::new(Bolt11OutgoingPaymentOptions {
                    bolt11: generate_fake_bolt11(signed_desc, cashu::Amount::ZERO),
                    max_fee_amount: None,
                    timeout_secs: None,
                    melt_options: None,
                })),
            )
            .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PaymentError::UnsupportedUnit));
    }

    #[tokio::test]
    async fn make_payment_alreadypaid() {
        let qid = Uuid::new_v4();
        let reqid = PaymentIdentifier::CustomId(qid.to_string());
        let onchain = Arc::new(MockOnChainWallet::new());
        let mut payrepo = MockPaymentRepository::new();
        let cloned_reqid = reqid.clone();
        let tkp = generate_random_keypair();
        payrepo
            .expect_load_outgoing()
            .with(eq(cloned_reqid.clone()))
            .returning(move |_| {
                Ok(payment::OutgoingRequest {
                    reqid: cloned_reqid.clone(),
                    amount: cashu::Amount::ZERO,
                    state: MeltQuoteState::Paid,
                })
            });
        let ebill = Arc::new(MockEBillNode::new());
        let interval = Duration::from_secs(1);
        let srvc = Service::new(
            onchain,
            Arc::new(payrepo),
            ebill,
            interval,
            tkp.x_only_public_key().0,
        )
        .await;
        let desc = wire_signatures::RequestToMeltDesc {
            qid,
            amount: cashu::Amount::ZERO,
        };
        let (content, signature) = serialize_n_schnorr_sign_borsh_msg(&desc, &tkp).unwrap();
        let signed_desc = wire_signatures::SignedRequestToMeltDesc { content, signature };
        let result = srvc
            .make_payment(
                &cashu::CurrencyUnit::Sat,
                OutgoingPaymentOptions::Bolt11(Box::new(Bolt11OutgoingPaymentOptions {
                    bolt11: generate_fake_bolt11(signed_desc, cashu::Amount::ZERO),
                    max_fee_amount: None,
                    timeout_secs: None,
                    melt_options: None,
                })),
            )
            .await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PaymentError::InvoiceAlreadyPaid
        ));
    }

    #[tokio::test]
    async fn make_payment_paid() {
        let qid = Uuid::new_v4();
        let reqid = PaymentIdentifier::CustomId(qid.to_string());
        let onchain = MockOnChainWallet::new();
        let mut payrepo = MockPaymentRepository::new();
        let tkp = generate_random_keypair();
        let cloned_reqid = reqid.clone();
        payrepo
            .expect_load_outgoing()
            .with(eq(reqid.clone()))
            .returning(move |_| {
                Ok(payment::OutgoingRequest {
                    reqid: cloned_reqid.clone(),
                    amount: cashu::Amount::ZERO,
                    state: MeltQuoteState::Pending,
                })
            });
        let cloned_reqid = reqid.clone();
        payrepo
            .expect_update_outgoing()
            .with(eq(payment::OutgoingRequest {
                reqid: cloned_reqid.clone(),
                amount: cashu::Amount::ZERO,
                state: MeltQuoteState::Paid,
            }))
            .returning(move |_| Ok(()));
        let ebill = MockEBillNode::new();
        let interval = Duration::from_secs(1);
        let srvc = Service::new(
            Arc::new(onchain),
            Arc::new(payrepo),
            Arc::new(ebill),
            interval,
            tkp.x_only_public_key().0,
        )
        .await;
        let desc = wire_signatures::RequestToMeltDesc {
            qid,
            amount: cashu::Amount::ZERO,
        };
        let (content, signature) = serialize_n_schnorr_sign_borsh_msg(&desc, &tkp).unwrap();
        let signed_desc = wire_signatures::SignedRequestToMeltDesc { content, signature };
        srvc.make_payment(
            &cashu::CurrencyUnit::Sat,
            OutgoingPaymentOptions::Bolt11(Box::new(Bolt11OutgoingPaymentOptions {
                bolt11: generate_fake_bolt11(signed_desc, cashu::Amount::ZERO),
                max_fee_amount: None,
                timeout_secs: None,
                melt_options: None,
            })),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn make_payment_quoteandrequestamountsdonotmatch() {
        let reqid = PaymentIdentifier::CustomId(Uuid::new_v4().to_string());
        let mut onchain = MockOnChainWallet::new();
        let tkp = generate_random_keypair();
        onchain.expect_network().returning(|| btc::Network::Testnet);
        let mut payrepo = MockPaymentRepository::new();
        let cloned_reqid = reqid.clone();
        payrepo
            .expect_load_outgoing()
            .with(eq(reqid.clone()))
            .returning(move |_| {
                Ok(payment::OutgoingRequest {
                    reqid: cloned_reqid.clone(),
                    amount: cashu::Amount::from(10),
                    state: MeltQuoteState::Pending,
                })
            });
        let ebill = MockEBillNode::new();
        let interval = Duration::from_secs(1);
        let srvc = Service::new(
            Arc::new(onchain),
            Arc::new(payrepo),
            Arc::new(ebill),
            interval,
            tkp.x_only_public_key().0,
        )
        .await;
        let desc = wire_signatures::RequestToMeltDesc {
            qid: Uuid::new_v4(),
            amount: cashu::Amount::from(10),
        };
        let (content, signature) = serialize_n_schnorr_sign_borsh_msg(&desc, &tkp).unwrap();
        let signed_desc = wire_signatures::SignedRequestToMeltDesc { content, signature };
        let result = srvc
            .make_payment(
                &cashu::CurrencyUnit::Sat,
                OutgoingPaymentOptions::Bolt11(Box::new(Bolt11OutgoingPaymentOptions {
                    bolt11: generate_fake_bolt11(signed_desc, cashu::Amount::from(11)),
                    max_fee_amount: None,
                    timeout_secs: None,
                    melt_options: None,
                })),
            )
            .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PaymentError::Amount(_)));
    }
}
