// ----- standard library imports
use std::{
    collections::HashMap,
    pin::Pin,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};
// ----- extra library imports
use anyhow::anyhow;
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
    {Amount, CurrencyUnit},
};
use futures::Stream;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
// ----- local imports
use crate::{
    error::{Error, Result},
    payment, TStamp,
};

// ----- end imports

type PaymentResult<T> = std::result::Result<T, PaymentError>;

#[async_trait]
pub trait OnChainWallet: Send + Sync {
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

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait PaymentRepository: Send + Sync {
    async fn load_incoming(&self, reqid: &PaymentIdentifier) -> Result<payment::IncomingRequest>;
    async fn store_incoming(&self, req: payment::IncomingRequest) -> Result<()>;
    async fn update_incoming(&self, req: payment::IncomingRequest) -> Result<()>;
    async fn list_unpaid_incoming_requests(&self) -> Result<Vec<payment::IncomingRequest>>;

    async fn load_outgoing(&self, reqid: &PaymentIdentifier) -> Result<payment::OutgoingRequest>;
    async fn store_outgoing(&self, req: payment::OutgoingRequest) -> Result<()>;
    async fn update_outgoing(&self, req: payment::OutgoingRequest) -> Result<()>;

    async fn store_foreign(&self, new: payment::ForeignPayment) -> Result<()>;
    async fn check_foreign_nonce(&self, nonce: &str) -> Result<Option<payment::ForeignPayment>>;
    async fn check_foreign_reqid(
        &self,
        reqid: &PaymentIdentifier,
    ) -> Result<Option<payment::ForeignPayment>>;
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
            ParsedDescription::ForeignECash(request) => {
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
            ParsedDescription::EbillRequestToPay(request) => {
                tracing::trace!("Parsed EBill request",);
                let message: wire_signatures::RequestToMintFromEBillDesc =
                    deserialize_borsh_msg(&request.content).map_err(Error::from)?;
                schnorr_verify_b64(&request.content, &request.signature, &self.treasury_pubkey)
                    .map_err(Error::from)?;
                let output = self
                    .ebill
                    .request_to_pay(&message.ebill_id, amount, message.deadline)
                    .await?;
                let recipient = self.onchain.add_descriptor(&output).await?;
                payment::PaymentType::EBill(recipient)
            }
            ParsedDescription::ClowderOnchain(uuid) => payment::PaymentType::ClowderOnchain(uuid),
            ParsedDescription::Dev => {
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
            ParsedDescription::None => {
                let recipient = self
                    .onchain
                    .generate_new_recipient()
                    .map_err(PaymentError::from)?;
                payment::PaymentType::OnChain(recipient)
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
        let uri = parse_to_bip21_uri(&description, self.onchain.network())?;
        let fees_btc = self.onchain.estimate_fees().await?;
        let fee = Amount::from(fees_btc.to_sat());
        let reqid = PaymentIdentifier::CustomId(Uuid::new_v4().to_string());
        let outgoing = payment::OutgoingRequest::new(reqid, uri, fees_btc)?;
        let response = PaymentQuoteResponse {
            request_lookup_id: Some(outgoing.reqid.clone()),
            amount: Amount::from(outgoing.amount.to_sat()),
            unit: CurrencyUnit::Sat,
            fee,
            state: outgoing.status,
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
        let reqid = PaymentIdentifier::CustomId(options.bolt11.description().to_string());
        let outgoing = self.payrepo.load_outgoing(&reqid).await;
        let mut request = match outgoing {
            Ok(request) => match request.status {
                MeltQuoteState::Paid => return Err(PaymentError::InvoiceAlreadyPaid),
                MeltQuoteState::Pending => return Err(PaymentError::InvoicePaymentPending),
                _ => request,
            },
            Err(Error::PaymentRequestNotFound(_)) => return Err(PaymentError::UnknownPaymentState),
            Err(e) => return Err(e.into()),
        };

        let quote_amount = btc::Amount::from_sat(
            options.bolt11.amount_milli_satoshis().unwrap_or_default() / 1000,
        );
        if request.amount != quote_amount {
            return Err(PaymentError::Amount(
                cdk_common::amount::Error::InvalidAmount(format!(
                    "melt_quote.amount {quote_amount} != request.amount {}",
                    request.amount
                )),
            ));
        }
        request.status = MeltQuoteState::Pending;
        self.payrepo.update_outgoing(request.clone()).await?;
        let (tx_id, total_fee) = self
            .onchain
            .send_to(
                request.recipient.clone(),
                request.amount,
                request.reserved_fees,
            )
            .await?;
        request.proof = Some(tx_id);
        let total_spent = request.amount + total_fee;
        request.total_spent = Some(total_spent);
        let store_result = self.payrepo.update_outgoing(request.clone()).await;
        if let Err(e) = store_result {
            tracing::error!(
                "Error in storing proof for reqid {}, tx_id {tx_id}, e: {e}",
                request.reqid
            );
        }

        let total_spent = Amount::from(total_spent.to_sat());
        let response = MakePaymentResponse {
            payment_lookup_id: reqid,
            payment_proof: Some(tx_id.to_string()),
            status: MeltQuoteState::Pending,
            total_spent,
            unit: CurrencyUnit::Sat,
        };

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

        if let Some(amount) = self.dev_payments.lock().unwrap().get(payment_identifier) {
            return Ok(vec![WaitPaymentResponse {
                payment_identifier: payment_identifier.clone(),
                payment_amount: *amount,
                unit: CurrencyUnit::Sat,
                payment_id: payment_identifier.to_string(),
            }]);
        }

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
        // nothing to look at, proceed
        let mut request = self.payrepo.load_incoming(payment_identifier).await?;
        if request.status == MintQuoteState::Unpaid {
            request.status = if let Some(recipient) = request.payment_type.recipient() {
                check_incoming_payment(&recipient, request.amount, self.onchain.as_ref()).await?
            } else {
                MintQuoteState::Paid
            };
            self.payrepo.update_incoming(request.clone()).await?;
        }
        if request.status == MintQuoteState::Paid {
            let payment_id = request
                .payment_type
                .recipient()
                .map(|a| a.to_string())
                .or_else(|| request.payment_type.clowder_quote().map(|u| u.to_string()))
                .unwrap_or_default();
            response.push(WaitPaymentResponse {
                payment_identifier: payment_identifier.clone(),
                payment_amount: cashu::Amount::from(request.amount.to_sat()),
                unit: CurrencyUnit::Sat,
                payment_id,
            });
        }
        Ok(response)
    }

    async fn check_outgoing_payment(
        &self,
        payment_identifier: &PaymentIdentifier,
    ) -> PaymentResult<MakePaymentResponse> {
        let _span = tracing::debug_span!("check_outgoing_payment", payment_identifier = %payment_identifier);

        let mut request = self.payrepo.load_outgoing(payment_identifier).await?;
        let total_spent = Amount::from(request.total_spent.unwrap_or(request.amount).to_sat());
        let response = MakePaymentResponse {
            payment_lookup_id: payment_identifier.clone(),
            payment_proof: request.proof.map(|txid| txid.to_string()),
            unit: CurrencyUnit::Sat,
            status: request.status,
            total_spent,
        };
        if matches!(request.status, MeltQuoteState::Paid) {
            return Ok(response);
        }

        let new_state = check_outgoing_payment(request.proof, self.onchain.as_ref()).await?;
        request.status = new_state;
        self.payrepo.update_outgoing(request).await?;
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

async fn check_outgoing_payment(
    tx_id: Option<btc::Txid>,
    onchain: &dyn OnChainWallet,
) -> Result<cdk_common::MeltQuoteState> {
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

fn parse_to_bip21_uri(input: &str, network: btc::Network) -> Result<bip21::Uri<'_>> {
    bip21::Uri::from_str(input)
        .map_err(|e| Error::Bip21Parse(anyhow!(e)))?
        .require_network(network)
        .map_err(|e| Error::Bip21Parse(anyhow!(e)))
}

enum ParsedDescription {
    Dev,
    EbillRequestToPay(wire_signatures::SignedRequestToMintFromEBillDesc),
    ForeignECash(web_exchange::RequestToMintFromForeigneCash),
    ClowderOnchain(Uuid),
    None,
}
impl ParsedDescription {
    fn parse(input: &str) -> Self {
        if let Ok(ebill_request) =
            serde_json::from_str::<wire_signatures::SignedRequestToMintFromEBillDesc>(input)
        {
            Self::EbillRequestToPay(ebill_request)
        } else if let Ok(foreign_ecash_request) =
            serde_json::from_str::<web_exchange::RequestToMintFromForeigneCash>(input)
        {
            Self::ForeignECash(foreign_ecash_request)
        } else if let Some(uuid_str) = input.strip_prefix("clowder:") {
            if let Ok(uuid) = Uuid::parse_str(uuid_str) {
                Self::ClowderOnchain(uuid)
            } else {
                Self::None
            }
        } else if input == "it's me, Mario" {
            Self::Dev
        } else {
            Self::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdk_wallet::bitcoin::hashes::{sha256, Hash};
    use cashu::lightning_invoice as ln;
    use cdk_common::payment::{Bolt11IncomingPaymentOptions, Bolt11OutgoingPaymentOptions};
    use mockall::predicate::*;

    fn generate_random_pubkey() -> btc::XOnlyPublicKey {
        let kp = btc::key::Keypair::new(secp256k1::global::SECP256K1, &mut rand::thread_rng());
        btc::XOnlyPublicKey::from_keypair(&kp).0
    }

    fn generate_random_address() -> btc::Address {
        let sk = btc::PrivateKey::generate(btc::Network::Testnet);
        let pk =
            btc::CompressedPublicKey::from_private_key(secp256k1::global::SECP256K1, &sk).unwrap();
        btc::Address::p2wpkh(&pk, btc::Network::Testnet)
    }

    fn generate_fake_bolt11(description: String, amount: cashu::Amount) -> ln::Bolt11Invoice {
        let payment_hash = sha256::Hash::from_slice(&[0; 32][..]).unwrap();
        let payment_secret = ln::PaymentSecret([42u8; 32]);
        let sk = secp256k1::SecretKey::new(&mut rand::thread_rng());
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
            fn generate_new_recipient(&self) -> Result<btc::Address>;
            fn network(&self) -> btc::Network;
            async fn add_descriptor(&self, descriptor: &str) -> Result<btc::Address>;
            async fn balance(&self) -> Result<bdk_wallet::Balance>;
            async fn get_address_balance(&self, recipient: &btc::Address) -> Result<btc::Amount>;
            async fn estimate_fees(&self) -> Result<btc::Amount>;
            async fn send_to(
                &self,
                recipient: btc::Address,
                amount: btc::Amount,
                max_fee: btc::Amount,
            ) -> Result<(btc::Txid, btc::Amount)>;
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
        let srvc = Service::new(onchain, payrepo, ebill, interval, generate_random_pubkey()).await;
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
    async fn create_incoming_payment_request_getnewaddress() {
        let address = generate_random_address();
        let cloned_address = address.clone();
        let mut onchain = MockOnChainWallet::new();
        onchain
            .expect_generate_new_recipient()
            .returning(move || Ok(address.clone()));
        let mut payrepo = MockPaymentRepository::new();
        payrepo.expect_store_incoming().returning(|_| Ok(()));
        let ebill = Arc::new(MockEBillNode::new());
        let interval = Duration::from_secs(1);
        let srvc = Service::new(
            Arc::new(onchain),
            Arc::new(payrepo),
            ebill,
            interval,
            generate_random_pubkey(),
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
            .await
            .unwrap();
        let uri: bip21::Uri = bip21::Uri::from_str(&result.request)
            .unwrap()
            .require_network(btc::Network::Testnet)
            .unwrap();
        assert_eq!(uri.address, cloned_address);
    }

    #[tokio::test]
    async fn make_payment_wrongunit() {
        let onchain = Arc::new(MockOnChainWallet::new());
        let payrepo = Arc::new(MockPaymentRepository::new());
        let ebill = Arc::new(MockEBillNode::new());
        let interval = Duration::from_secs(1);
        let srvc = Service::new(onchain, payrepo, ebill, interval, generate_random_pubkey()).await;
        let result = srvc
            .make_payment(
                &cashu::CurrencyUnit::Usd,
                OutgoingPaymentOptions::Bolt11(Box::new(Bolt11OutgoingPaymentOptions {
                    bolt11: generate_fake_bolt11(String::default(), cashu::Amount::ZERO),
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
        let reqid = PaymentIdentifier::CustomId(Uuid::new_v4().to_string());
        let onchain = Arc::new(MockOnChainWallet::new());
        let mut payrepo = MockPaymentRepository::new();
        let cloned_reqid = reqid.clone();
        payrepo
            .expect_load_outgoing()
            .with(eq(cloned_reqid.clone()))
            .returning(move |_| {
                Ok(payment::OutgoingRequest {
                    reqid: cloned_reqid.clone(),
                    reserved_fees: btc::Amount::ZERO,
                    amount: btc::Amount::ZERO,
                    recipient: generate_random_address(),
                    status: MeltQuoteState::Paid,
                    proof: None,
                    total_spent: None,
                })
            });
        let ebill = Arc::new(MockEBillNode::new());
        let interval = Duration::from_secs(1);
        let srvc = Service::new(
            onchain,
            Arc::new(payrepo),
            ebill,
            interval,
            generate_random_pubkey(),
        )
        .await;
        let result = srvc
            .make_payment(
                &cashu::CurrencyUnit::Sat,
                OutgoingPaymentOptions::Bolt11(Box::new(Bolt11OutgoingPaymentOptions {
                    bolt11: generate_fake_bolt11(reqid.to_string(), Amount::ZERO),
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
    async fn make_payment_pending() {
        let reqid = PaymentIdentifier::CustomId(Uuid::new_v4().to_string());
        let onchain = MockOnChainWallet::new();
        let mut payrepo = MockPaymentRepository::new();
        let cloned_reqid = reqid.clone();
        payrepo
            .expect_load_outgoing()
            .with(eq(reqid.clone()))
            .returning(move |_| {
                Ok(payment::OutgoingRequest {
                    reqid: cloned_reqid.clone(),
                    amount: btc::Amount::ZERO,
                    recipient: generate_random_address(),
                    status: MeltQuoteState::Pending,
                    proof: None,
                    total_spent: None,
                    reserved_fees: btc::Amount::ZERO,
                })
            });
        let ebill = MockEBillNode::new();
        let interval = Duration::from_secs(1);
        let srvc = Service::new(
            Arc::new(onchain),
            Arc::new(payrepo),
            Arc::new(ebill),
            interval,
            generate_random_pubkey(),
        )
        .await;
        let result = srvc
            .make_payment(
                &cashu::CurrencyUnit::Sat,
                OutgoingPaymentOptions::Bolt11(Box::new(Bolt11OutgoingPaymentOptions {
                    bolt11: generate_fake_bolt11(reqid.to_string(), Amount::ZERO),
                    max_fee_amount: None,
                    timeout_secs: None,
                    melt_options: None,
                })),
            )
            .await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PaymentError::InvoicePaymentPending
        ));
    }

    #[tokio::test]
    async fn make_payment_quoteandrequestamountsdonotmatch() {
        let reqid = PaymentIdentifier::CustomId(Uuid::new_v4().to_string());
        let mut onchain = MockOnChainWallet::new();
        onchain.expect_network().returning(|| btc::Network::Testnet);
        let mut payrepo = MockPaymentRepository::new();
        let cloned_reqid = reqid.clone();
        payrepo
            .expect_load_outgoing()
            .with(eq(reqid.clone()))
            .returning(move |_| {
                Ok(payment::OutgoingRequest {
                    reqid: cloned_reqid.clone(),
                    amount: btc::Amount::from_sat(10),
                    recipient: generate_random_address(),
                    status: MeltQuoteState::Unpaid,
                    proof: None,
                    total_spent: None,
                    reserved_fees: btc::Amount::ZERO,
                })
            });
        let ebill = MockEBillNode::new();
        let interval = Duration::from_secs(1);
        let srvc = Service::new(
            Arc::new(onchain),
            Arc::new(payrepo),
            Arc::new(ebill),
            interval,
            generate_random_pubkey(),
        )
        .await;
        let result = srvc
            .make_payment(
                &cashu::CurrencyUnit::Sat,
                OutgoingPaymentOptions::Bolt11(Box::new(Bolt11OutgoingPaymentOptions {
                    bolt11: generate_fake_bolt11(reqid.to_string(), cashu::Amount::from(11)),
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
