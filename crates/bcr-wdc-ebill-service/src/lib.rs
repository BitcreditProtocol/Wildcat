// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{get, post, put},
    Router,
};
use bcr_ebill_api::{
    external::bitcoin::BitcoinClient,
    service::{
        bill_service::{service::BillService, BillServiceApi},
        contact_service::{ContactService, ContactServiceApi},
        identity_service::{IdentityService, IdentityServiceApi},
        notification_service::{create_notification_service, NostrClient},
    },
};
use bcr_ebill_transport::{NotificationServiceApi, PushApi, PushService};
// ----- local modules
mod error;
mod web;
// ----- end imports

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    pub ebill_db: ConnectionConfig,
    pub bitcoin_network: String,
    pub esplora_base_url: String,
    pub nostr_cfg: NostrConfig,
    pub data_dir: String,
    pub job_runner_initial_delay_seconds: u64,
    pub job_runner_check_interval_seconds: u64,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct NostrConfig {
    pub only_known_contacts: bool,
    pub relays: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    pub contact_service: Arc<dyn ContactServiceApi>,
    pub bill_service: Arc<dyn BillServiceApi>,
    pub identity_service: Arc<dyn IdentityServiceApi>,
    pub notification_service: Arc<dyn NotificationServiceApi>,
    pub push_service: Arc<dyn PushApi>,
}

impl AppController {
    pub async fn new(
        cfg: bcr_ebill_api::Config,
        nostr_clients: Vec<Arc<NostrClient>>,
        db: bcr_ebill_api::DbContext,
    ) -> Self {
        let contact_service = Arc::new(ContactService::new(
            db.contact_store.clone(),
            db.file_upload_store.clone(),
            db.identity_store.clone(),
            db.nostr_contact_store.clone(),
            &cfg.clone(),
        ));

        let notification_service = create_notification_service(
            nostr_clients,
            db.notification_store.clone(),
            contact_service.clone(),
            db.queued_message_store.clone(),
            cfg.nostr_config.relays.to_owned(),
        )
        .await
        .expect("Failed to create notification service");

        let bill_service = Arc::new(BillService::new(
            db.bill_store.clone(),
            db.bill_blockchain_store.clone(),
            db.identity_store.clone(),
            db.file_upload_store.clone(),
            Arc::new(BitcoinClient::new()),
            notification_service.clone(),
            db.identity_chain_store.clone(),
            db.company_chain_store.clone(),
            db.contact_store.clone(),
            db.company_store.clone(),
        ));

        let identity_service = IdentityService::new(
            db.identity_store.clone(),
            db.file_upload_store.clone(),
            db.identity_chain_store.clone(),
        );

        let push_service = Arc::new(PushService::new());

        Self {
            contact_service,
            bill_service,
            identity_service: Arc::new(identity_service),
            notification_service,
            push_service,
        }
    }
}

pub fn routes(ctrl: AppController) -> Router {
    Router::new()
        .route("/identity/detail", get(web::get_identity))
        .route("/identity/create", post(web::create_identity))
        .route("/identity/seed/backup", get(web::get_seed_phrase))
        .route("/identity/seed/recover", put(web::recover_from_seed_phrase))
        .route("/contact/create", post(web::create_contact))
        .route("/bill/list", get(web::get_bills))
        .route("/bill/detail/{bill_id}", get(web::get_bill_detail))
        .route(
            "/bill/endorsements/{bill_id}",
            get(web::get_bill_endorsements),
        )
        .route(
            "/bill/attachment/{bill_id}/{file_name}",
            get(web::get_bill_attachment),
        )
        .route("/bill/request_to_pay", put(web::request_to_pay_bill))
        .route("/bill/bitcoin_key/{bill_id}", get(web::bill_bitcoin_key))
        .with_state(ctrl)
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;
    use async_broadcast::Receiver;
    use async_trait::async_trait;
    use bcr_ebill_api::{
        data::{
            bill::{
                BillAction, BillCombinedBitcoinKey, BillIssueData, BillKeys, BillsBalanceOverview,
                BillsFilterRole, BitcreditBill, BitcreditBillResult, Endorsement,
                LightBitcreditBillResult, PastEndorsee, PastPaymentResult,
            },
            contact::{BillIdentParticipant, BillParticipant, Contact, ContactType},
            identity::{ActiveIdentityState, Identity, IdentityType, IdentityWithAll},
            notification::{ActionType, Notification},
            File, OptionalPostalAddress, PostalAddress,
        },
        service::{
            bill_service::Error as BillError, bill_service::Result as BillResult, Error, Result,
        },
        util::BcrKeys,
        NotificationFilter,
    };
    use bcr_ebill_core::{blockchain::bill::BillBlockchain, ServiceTraitBounds};
    use bcr_ebill_transport::{event::chain_event::BillChainEvent, Result as NotifResult};
    use std::collections::HashMap;

    mockall::mock! {
        pub BillServiceApi {}

        impl ServiceTraitBounds for BillServiceApi {}

        #[async_trait]
        impl BillServiceApi for BillServiceApi {
            async fn get_bill_balances(
                &self,
                currency: &str,
                current_identity_node_id: &str,
            ) -> BillResult<BillsBalanceOverview>;
            async fn search_bills(
                &self,
                currency: &str,
                search_term: &Option<String>,
                date_range_from: Option<u64>,
                date_range_to: Option<u64>,
                role: &BillsFilterRole,
                current_identity_node_id: &str,
            ) -> BillResult<Vec<LightBitcreditBillResult>>;
            async fn get_bills(&self, current_identity_node_id: &str) -> BillResult<Vec<BitcreditBillResult>>;
            async fn get_combined_bitcoin_key_for_bill(
                &self,
                bill_id: &str,
                caller_public_data: &BillParticipant,
                caller_keys: &BcrKeys,
            ) -> BillResult<BillCombinedBitcoinKey>;
            async fn get_detail(
                &self,
                bill_id: &str,
                local_identity: &Identity,
                current_identity_node_id: &str,
                current_timestamp: u64,
            ) -> BillResult<BitcreditBillResult>;
            async fn get_bill_keys(&self, bill_id: &str) -> BillResult<BillKeys>;
            async fn open_and_decrypt_attached_file(
                &self,
                bill_id: &str,
                file_name: &str,
                bill_private_key: &str,
            ) -> BillResult<Vec<u8>>;
            async fn encrypt_and_save_uploaded_file(
                &self,
                file_name: &str,
                file_bytes: &[u8],
                bill_id: &str,
                bill_public_key: &str,
            ) -> BillResult<File>;
            async fn issue_new_bill(&self, data: BillIssueData) -> BillResult<BitcreditBill>;
            async fn execute_bill_action(
                &self,
                bill_id: &str,
                bill_action: BillAction,
                signer_public_data: &BillParticipant,
                signer_keys: &BcrKeys,
                timestamp: u64,
            ) -> BillResult<BillBlockchain>;
            async fn check_bills_payment(&self) -> BillResult<()>;
            async fn check_payment_for_bill(&self, bill_id: &str, identity: &Identity) -> BillResult<()>;
            async fn check_bills_offer_to_sell_payment(&self) -> BillResult<()>;
            async fn check_offer_to_sell_payment_for_bill(
                &self,
                bill_id: &str,
                identity: &IdentityWithAll,
            ) -> BillResult<()>;
            async fn check_bills_in_recourse_payment(&self) -> BillResult<()>;
            async fn check_recourse_payment_for_bill(
                &self,
                bill_id: &str,
                identity: &IdentityWithAll,
            ) -> BillResult<()>;
            async fn check_bills_timeouts(&self, now: u64) -> BillResult<()>;
            async fn get_past_endorsees(
                &self,
                bill_id: &str,
                current_identity_node_id: &str,
            ) -> BillResult<Vec<PastEndorsee>>;
            async fn get_past_payments(
                &self,
                bill_id: &str,
                caller_public_data: &BillParticipant,
                caller_keys: &BcrKeys,
                timestamp: u64,
            ) -> BillResult<Vec<PastPaymentResult>>;
            async fn get_endorsements(
                &self,
                bill_id: &str,
                current_identity_node_id: &str,
            ) -> BillResult<Vec<Endorsement>>;
            async fn clear_bill_cache(&self) -> BillResult<()>;
        }
    }

    mockall::mock! {
        pub PushApi {}

        impl ServiceTraitBounds for PushApi {}

        #[async_trait]
        impl PushApi for PushApi {
            async fn send(&self, value: serde_json::Value);
            async fn subscribe(&self) -> Receiver<serde_json::Value>;
        }
    }

    mockall::mock! {
        pub ContactServiceApi {}

        impl ServiceTraitBounds for ContactServiceApi {}

        #[async_trait]
        impl ContactServiceApi for ContactServiceApi {
            async fn search(&self, search_term: &str) -> Result<Vec<Contact>>;
            async fn get_contacts(&self) -> Result<Vec<Contact>>;
            async fn get_contact(&self, node_id: &str) -> Result<Contact>;
            async fn get_identity_by_node_id(&self, node_id: &str) -> Result<Option<BillParticipant>>;
            async fn delete(&self, node_id: &str) -> Result<()>;
            async fn update_contact(
                &self,
                node_id: &str,
                name: Option<String>,
                email: Option<String>,
                postal_address: OptionalPostalAddress,
                date_of_birth_or_registration: Option<String>,
                country_of_birth_or_registration: Option<String>,
                city_of_birth_or_registration: Option<String>,
                identification_number: Option<String>,
                avatar_file_upload_id: Option<String>,
                proof_document_file_upload_id: Option<String>,
            ) -> Result<()>;
            async fn add_contact(
                &self,
                node_id: &str,
                t: ContactType,
                name: String,
                email: Option<String>,
                postal_address: Option<PostalAddress>,
                date_of_birth_or_registration: Option<String>,
                country_of_birth_or_registration: Option<String>,
                city_of_birth_or_registration: Option<String>,
                identification_number: Option<String>,
                avatar_file_upload_id: Option<String>,
                proof_document_file_upload_id: Option<String>,
            ) -> Result<Contact>;
            async fn deanonymize_contact(
                &self,
                node_id: &str,
                t: ContactType,
                name: String,
                email: Option<String>,
                postal_address: Option<PostalAddress>,
                date_of_birth_or_registration: Option<String>,
                country_of_birth_or_registration: Option<String>,
                city_of_birth_or_registration: Option<String>,
                identification_number: Option<String>,
                avatar_file_upload_id: Option<String>,
                proof_document_file_upload_id: Option<String>,
            ) -> Result<Contact>;
            async fn is_known_npub(&self, npub: &bcr_ebill_api::service::contact_service::PublicKey) -> Result<bool>;
            async fn open_and_decrypt_file(
                &self,
                id: &str,
                file_name: &str,
                private_key: &str,
            ) -> Result<Vec<u8>>;
        }
    }

    mockall::mock! {
        pub IdentityServiceApi {}


        impl ServiceTraitBounds for IdentityServiceApi {}

        #[async_trait]
        impl IdentityServiceApi for IdentityServiceApi {
            async fn update_identity(
                &self,
                name: Option<String>,
                email: Option<String>,
                postal_address: OptionalPostalAddress,
                date_of_birth: Option<String>,
                country_of_birth: Option<String>,
                city_of_birth: Option<String>,
                identification_number: Option<String>,
                profile_picture_file_upload_id: Option<String>,
                identity_document_file_upload_id: Option<String>,
                timestamp: u64,
            ) -> Result<()>;
            async fn get_full_identity(&self) -> Result<IdentityWithAll>;
            async fn get_identity(&self) -> Result<Identity>;
            async fn identity_exists(&self) -> bool;
            async fn create_identity(
                &self,
                t: IdentityType,
                name: String,
                email: Option<String>,
                postal_address: OptionalPostalAddress,
                date_of_birth: Option<String>,
                country_of_birth: Option<String>,
                city_of_birth: Option<String>,
                identification_number: Option<String>,
                profile_picture_file_upload_id: Option<String>,
                identity_document_file_upload_id: Option<String>,
                timestamp: u64,
            ) -> Result<()>;
            async fn deanonymize_identity(
                &self,
                t: IdentityType,
                name: String,
                email: Option<String>,
                postal_address: OptionalPostalAddress,
                date_of_birth: Option<String>,
                country_of_birth: Option<String>,
                city_of_birth: Option<String>,
                identification_number: Option<String>,
                profile_picture_file_upload_id: Option<String>,
                identity_document_file_upload_id: Option<String>,
                timestamp: u64,
            ) -> Result<()>;
            async fn get_seedphrase(&self) -> Result<String>;
            async fn recover_from_seedphrase(&self, seed: &str) -> Result<()>;
            async fn open_and_decrypt_file(
                &self,
                id: &str,
                file_name: &str,
                private_key: &str,
            ) -> Result<Vec<u8>>;
            async fn get_current_identity(&self) -> Result<ActiveIdentityState>;
            async fn set_current_personal_identity(&self, node_id: &str) -> Result<()>;
            async fn set_current_company_identity(&self, node_id: &str) -> Result<()>;
        }
    }
    mockall::mock! {
        pub NotificationServiceApi {}

        impl ServiceTraitBounds for NotificationServiceApi {}

        #[async_trait]
        impl NotificationServiceApi for NotificationServiceApi {
            async fn send_bill_is_signed_event(&self, event: &BillChainEvent) -> NotifResult<()>;
            async fn send_bill_is_accepted_event(&self, event: &BillChainEvent) -> NotifResult<()>;
            async fn send_request_to_accept_event(&self, event: &BillChainEvent) -> NotifResult<()>;
            async fn send_request_to_pay_event(&self, event: &BillChainEvent) -> NotifResult<()>;
            async fn send_bill_is_paid_event(&self, event: &BillChainEvent) -> NotifResult<()>;
            async fn send_bill_is_endorsed_event(&self, event: &BillChainEvent) -> NotifResult<()>;
            async fn send_offer_to_sell_event(
                &self,
                event: &BillChainEvent,
                buyer: &BillParticipant,
            ) -> NotifResult<()>;
            async fn send_bill_is_sold_event(
                &self,
                event: &BillChainEvent,
                buyer: &BillParticipant,
            ) -> NotifResult<()>;
            async fn send_bill_recourse_paid_event(
                &self,
                event: &BillChainEvent,
                recoursee: &BillIdentParticipant,
            ) -> NotifResult<()>;
            async fn send_request_to_action_rejected_event(
                &self,
                event: &BillChainEvent,
                rejected_action: ActionType,
            ) -> NotifResult<()>;
            async fn send_request_to_action_timed_out_event(
                &self,
                sender_node_id: &str,
                bill_id: &str,
                sum: Option<u64>,
                timed_out_action: ActionType,
                recipients: Vec<BillParticipant>,
            ) -> NotifResult<()>;
            async fn send_recourse_action_event(
                &self,
                event: &BillChainEvent,
                action: ActionType,
                recoursee: &BillIdentParticipant,
            ) -> NotifResult<()>;
            async fn send_request_to_mint_event(
                &self,
                sender_node_id: &str,
                bill: &BitcreditBill,
            ) -> NotifResult<()>;
            async fn send_new_quote_event(&self, quote: &BitcreditBill) -> NotifResult<()>;
            async fn send_quote_is_approved_event(&self, quote: &BitcreditBill) -> NotifResult<()>;
            async fn get_client_notifications(
                &self,
                filter: NotificationFilter,
            ) -> NotifResult<Vec<Notification>>;
            async fn mark_notification_as_done(&self, notification_id: &str) -> NotifResult<()>;
            async fn get_active_bill_notification(&self, bill_id: &str) -> Option<Notification>;
            async fn get_active_bill_notifications(
                &self,
                bill_ids: &[String],
            ) -> HashMap<String, Notification>;
            async fn check_bill_notification_sent(
                &self,
                bill_id: &str,
                block_height: i32,
                action: ActionType,
            ) -> NotifResult<bool>;
            async fn mark_bill_notification_sent(
                &self,
                bill_id: &str,
                block_height: i32,
                action: ActionType,
            ) -> NotifResult<()>;
            async fn send_retry_messages(&self) -> NotifResult<()>;
        }
    }

    impl std::default::Default for AppController {
        fn default() -> Self {
            let mut mock_contact_service = MockContactServiceApi::new();
            mock_contact_service.expect_add_contact().returning(
                |_, _, _, _, _, _, _, _, _, _, _| {
                    Err(Error::Validation(
                        bcr_ebill_core::ValidationError::FieldEmpty(bcr_ebill_core::Field::Name),
                    ))
                },
            );
            let mut mock_bill_service = MockBillServiceApi::new();
            mock_bill_service
                .expect_get_bill_keys()
                .returning(|_| Err(BillError::NotFound));
            mock_bill_service
                .expect_open_and_decrypt_attached_file()
                .returning(|_, _, _| Err(BillError::NotFound));
            let mock_notification_service = MockNotificationServiceApi::new();
            let mock_push_api = MockPushApi::new();
            let mut mock_identity_service = MockIdentityServiceApi::new();
            mock_identity_service
                .expect_get_identity()
                .returning(|| Err(Error::NotFound));
            mock_identity_service
                .expect_get_full_identity()
                .returning(|| Err(Error::NotFound));
            mock_identity_service
                .expect_recover_from_seedphrase()
                .returning(|_| Ok(()));
            mock_identity_service
                .expect_get_seedphrase()
                .returning(|| Ok(bip39::Mnemonic::generate(12).unwrap().to_string()));
            mock_identity_service
                .expect_identity_exists()
                .returning(|| true);
            Self {
                contact_service: Arc::new(mock_contact_service),
                bill_service: Arc::new(mock_bill_service),
                identity_service: Arc::new(mock_identity_service),
                notification_service: Arc::new(mock_notification_service),
                push_service: Arc::new(mock_push_api),
            }
        }
    }

    pub fn build_test_server() -> axum_test::TestServer {
        let cfg = axum_test::TestServerConfig {
            transport: Some(axum_test::Transport::HttpRandomPort),
            ..Default::default()
        };
        let cntrl = AppController::default();
        axum_test::TestServer::new_with_config(routes(cntrl), cfg)
            .expect("failed to start test server")
    }
}
