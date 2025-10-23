// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{get, post, put},
    Router,
};
use bcr_ebill_api::{
    external::{
        bitcoin::BitcoinClient, court::CourtClient, email::EmailClient,
        file_storage::FileStorageClient, mint::MintClient,
    },
    service::{
        bill_service::{BillService, BillServiceApi},
        contact_service::{ContactService, ContactServiceApi},
        identity_service::{IdentityService, IdentityServiceApi},
        notification_service::NotificationServiceApi,
    },
};
use bcr_ebill_transport::{create_notification_service, NostrClient, PushApi, PushService};
// ----- local modules
mod error;
mod web;
// ----- end imports

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    pub ebill_db: ConnectionConfig,
    pub bitcoin_network: String,
    pub esplora_base_url: url::Url,
    pub nostr_cfg: NostrConfig,
    pub mint_config: MintConfig,
    pub payment_config: PaymentConfig,
    pub data_dir: String,
    pub job_runner_initial_delay_seconds: u64,
    pub job_runner_check_interval_seconds: u64,
    pub url: url::Url,
    pub court_config: CourtConfig,
    pub dev_mode_config: DevModeConfig,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct NostrConfig {
    pub only_known_contacts: bool,
    pub relays: Vec<url::Url>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct MintConfig {
    pub default_mint_url: url::Url,
    pub default_mint_node_id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ConnectionConfig {
    pub connection: String,
    pub namespace: String,
    pub database: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PaymentConfig {
    pub num_confirmations_for_payment: usize,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CourtConfig {
    /// The default court URL
    pub default_url: url::Url,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DevModeConfig {
    /// Whether dev mode is on
    pub on: bool,
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
        let email_client = Arc::new(EmailClient::new());
        let notification_service = create_notification_service(
            nostr_clients,
            db.clone(),
            email_client,
            cfg.nostr_config.relays.to_owned(),
        )
        .await
        .expect("Failed to create notification service");
        let file_upload_client = Arc::new(FileStorageClient::new());
        let contact_service = Arc::new(ContactService::new(
            db.contact_store.clone(),
            db.file_upload_store.clone(),
            file_upload_client.clone(),
            db.identity_store.clone(),
            db.nostr_contact_store.clone(),
            notification_service.clone(),
            &cfg.clone(),
        ));

        let court_client = Arc::new(CourtClient::new());
        let bill_service = Arc::new(BillService::new(
            db.bill_store.clone(),
            db.bill_blockchain_store.clone(),
            db.identity_store.clone(),
            db.file_upload_store.clone(),
            file_upload_client.clone(),
            Arc::new(BitcoinClient::new()),
            notification_service.clone(),
            db.identity_chain_store.clone(),
            db.company_chain_store.clone(),
            db.contact_store.clone(),
            db.company_store.clone(),
            db.mint_store.clone(),
            Arc::new(MintClient::new()),
            court_client.clone(),
            db.nostr_contact_store.clone(),
        ));

        let identity_service = IdentityService::new(
            db.identity_store.clone(),
            db.file_upload_store.clone(),
            file_upload_client.clone(),
            db.identity_chain_store.clone(),
            notification_service.clone(),
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
        .route("/v1/admin/identity/detail", get(web::get_identity))
        .route("/v1/admin/identity/create", post(web::create_identity))
        .route("/v1/admin/identity/seed/backup", get(web::get_seed_phrase))
        .route(
            "/v1/admin/identity/seed/recover",
            put(web::recover_from_seed_phrase),
        )
        .route("/v1/admin/bill/list", get(web::get_bills))
        .route("/v1/admin/bill/detail/{bill_id}", get(web::get_bill_detail))
        .route(
            "/v1/admin/bill/payment_status/{bill_id}",
            get(web::get_bill_payment_status),
        )
        .route(
            "/v1/admin/bill/endorsements/{bill_id}",
            get(web::get_bill_endorsements),
        )
        .route(
            "/v1/admin/bill/attachment/{bill_id}/{file_name}",
            get(web::get_bill_attachment),
        )
        .route(
            "/v1/admin/bill/request_to_pay",
            put(web::request_to_pay_bill),
        )
        .route(
            "/v1/admin/bill/bitcoin_key/{bill_id}",
            get(web::bill_bitcoin_key),
        )
        .route(
            "/v1/admin/bill/validate_and_decrypt_shared_bill",
            post(web::validate_and_decrypt_shared_bill),
        )
        .route(
            "/v1/admin/ebill/get_file_from_request_to_mint",
            get(web::get_encrypted_bill_file_from_request_to_mint),
        )
        .with_state(ctrl)
}

#[cfg(feature = "test-utils")]
pub mod test_utils {
    use super::*;
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
            mint::MintRequestState,
            notification::{ActionType, Notification},
            File, OptionalPostalAddress, PostalAddress,
        },
        service::notification_service::NostrContactData,
        service::{
            bill_service::Error as BillError,
            bill_service::Result as BillResult,
            notification_service::event::{BillChainEvent, CompanyChainEvent, IdentityChainEvent},
            Error, Result,
        },
        util::BcrKeys,
        NotificationFilter,
    };
    use bcr_ebill_core::{blockchain::bill::BillBlockchain, ServiceTraitBounds};
    use bcr_ebill_core::{
        city::City, country::Country, date::Date, email::Email, identification::Identification,
        name::Name,
    };
    use bcr_ebill_transport::Result as NotifResult;
    use std::collections::HashMap;

    mockall::mock! {
        pub BillServiceApi {}

        impl ServiceTraitBounds for BillServiceApi {}

        #[async_trait]
        impl BillServiceApi for BillServiceApi {
            async fn get_bill_history(
                &self,
                bill_id: &bcr_ebill_core::bill::BillId,
                local_identity: &bcr_ebill_core::identity::Identity,
                current_identity_node_id: &bcr_ebill_core::NodeId,
                current_timestamp: u64,
            ) -> BillResult<bcr_ebill_core::bill::BillHistory>;
            async fn share_bill_with_court(
                &self,
                bill_id: &bcr_ebill_core::bill::BillId,
                signer_public_data: &BillParticipant,
                signer_keys: &BcrKeys,
                court_node_id: &bcr_ebill_core::NodeId,
            ) -> BillResult<()>;
            async fn dev_mode_get_full_bill_chain(
                &self,
                bill_id: &bcr_ebill_core::bill::BillId,
                current_identity_node_id: &bcr_ebill_core::NodeId,
            ) -> BillResult<Vec<bcr_ebill_core::blockchain::bill::chain::BillBlockPlaintextWrapper>>;
            async fn get_bill_balances(
                &self,
                currency: &str,
                current_identity_node_id: &bcr_ebill_core::NodeId,
            ) -> BillResult<BillsBalanceOverview>;
            async fn search_bills(
                &self,
                currency: &str,
                search_term: &Option<String>,
                date_range_from: Option<u64>,
                date_range_to: Option<u64>,
                role: &BillsFilterRole,
                current_identity_node_id: &bcr_ebill_core::NodeId,
            ) -> BillResult<Vec<LightBitcreditBillResult>>;
            async fn get_bills(&self, current_identity_node_id: &bcr_ebill_core::NodeId) -> BillResult<Vec<BitcreditBillResult>>;
            async fn get_combined_bitcoin_key_for_bill(
                &self,
                bill_id: &bcr_ebill_core::bill::BillId,
                caller_public_data: &BillParticipant,
                caller_keys: &BcrKeys,
            ) -> BillResult<BillCombinedBitcoinKey>;
            async fn get_detail(
                &self,
                bill_id: &bcr_ebill_core::bill::BillId,
                local_identity: &Identity,
                current_identity_node_id: &bcr_ebill_core::NodeId,
                current_timestamp: u64,
            ) -> BillResult<BitcreditBillResult>;
            async fn get_bill_keys(&self, bill_id: &bcr_ebill_core::bill::BillId) -> BillResult<BillKeys>;
            async fn open_and_decrypt_attached_file(
                &self,
                bill_id: &bcr_ebill_core::bill::BillId,
                file: &File,
                bill_private_key: &bcr_ebill_core::SecretKey,
            ) -> BillResult<Vec<u8>>;
            async fn issue_new_bill(&self, data: BillIssueData) -> BillResult<BitcreditBill>;
            async fn execute_bill_action(
                &self,
                bill_id: &bcr_ebill_core::bill::BillId,
                bill_action: BillAction,
                signer_public_data: &BillParticipant,
                signer_keys: &BcrKeys,
                timestamp: u64,
            ) -> BillResult<BillBlockchain>;
            async fn check_bills_payment(&self) -> BillResult<()>;
            async fn check_payment_for_bill(&self, bill_id: &bcr_ebill_core::bill::BillId, identity: &Identity) -> BillResult<()>;
            async fn check_bills_offer_to_sell_payment(&self) -> BillResult<()>;
            async fn check_offer_to_sell_payment_for_bill(
                &self,
                bill_id: &bcr_ebill_core::bill::BillId,
                identity: &IdentityWithAll,
            ) -> BillResult<()>;
            async fn check_bills_in_recourse_payment(&self) -> BillResult<()>;
            async fn check_recourse_payment_for_bill(
                &self,
                bill_id: &bcr_ebill_core::bill::BillId,
                identity: &IdentityWithAll,
            ) -> BillResult<()>;
            async fn check_bills_timeouts(&self, now: u64) -> BillResult<()>;
            async fn get_past_endorsees(
                &self,
                bill_id: &bcr_ebill_core::bill::BillId,
                current_identity_node_id: &bcr_ebill_core::NodeId,
            ) -> BillResult<Vec<PastEndorsee>>;
            async fn get_past_payments(
                &self,
                bill_id: &bcr_ebill_core::bill::BillId,
                caller_public_data: &BillParticipant,
                caller_keys: &BcrKeys,
                timestamp: u64,
            ) -> BillResult<Vec<PastPaymentResult>>;
            async fn get_endorsements(
                &self,
                bill_id: &bcr_ebill_core::bill::BillId,
                identity: &bcr_ebill_core::identity::Identity,
                current_identity_node_id: &bcr_ebill_core::NodeId,
                current_timestamp: u64,
            ) -> BillResult<Vec<Endorsement>>;
            async fn clear_bill_cache(&self) -> BillResult<()>;
            async fn request_to_mint(
                &self,
                bill_id: &bcr_ebill_core::bill::BillId,
                mint_node_id: &bcr_ebill_core::NodeId,
                signer_public_data: &BillParticipant,
                signer_keys: &BcrKeys,
                timestamp: u64,
            ) -> BillResult<()>;
            async fn get_mint_state(
                &self,
                bill_id: &bcr_ebill_core::bill::BillId,
                current_identity_node_id: &bcr_ebill_core::NodeId,
            ) -> BillResult<Vec<MintRequestState>>;
            async fn cancel_request_to_mint(
                &self,
                mint_request_id: &str,
                current_identity_node_id: &bcr_ebill_core::NodeId,
            ) -> BillResult<()>;
            async fn accept_mint_offer(
                &self,
                mint_request_id: &str,
                signer_public_data: &BillParticipant,
                signer_keys: &BcrKeys,
                timestamp: u64,
            ) -> BillResult<()>;
            async fn reject_mint_offer(
                &self,
                mint_request_id: &str,
                current_identity_node_id: &bcr_ebill_core::NodeId,
            ) -> BillResult<()>;
            async fn check_mint_state(&self, bill_id: &bcr_ebill_core::bill::BillId, current_identity_node_id: &bcr_ebill_core::NodeId) -> BillResult<()>;
            async fn check_mint_state_for_all_bills(&self) -> BillResult<()>;
        }
    }

    mockall::mock! {
        pub PushApi {}

        impl ServiceTraitBounds for PushApi {}

        #[async_trait]
        impl PushApi for PushApi {
            async fn send(&self, value: serde_json::Value);
            async fn subscribe(&self) -> bcr_ebill_transport::Receiver<serde_json::Value>;
        }
    }

    mockall::mock! {
            pub ContactServiceApi {}

            impl ServiceTraitBounds for ContactServiceApi {}

            #[async_trait]
            impl ContactServiceApi for ContactServiceApi {
                async fn search(&self, search_term: &str,include_logical: Option<bool>,        include_contact: Option<bool>,) -> Result<Vec<Contact>>;
                async fn get_contacts(&self) -> Result<Vec<Contact>>;
                async fn get_contact(&self, node_id: &bcr_ebill_core::NodeId) -> Result<Contact>;
                async fn get_identity_by_node_id(&self, node_id: &bcr_ebill_core::NodeId) -> Result<Option<BillParticipant>>;
                async fn delete(&self, node_id: &bcr_ebill_core::NodeId) -> Result<()>;
                async fn update_contact(
                    &self,
                    node_id: &bcr_ebill_core::NodeId,
                    name: Option<Name>,
                    email: Option<Email>,
                    postal_address: OptionalPostalAddress,
                    date_of_birth_or_registration: Option<Date>,
                    country_of_birth_or_registration: Option<Country>,
                    city_of_birth_or_registration: Option<City>,
                    identification_number: Option<Identification>,
                    avatar_file_upload_id: Option<String>,
    ignore_avatar_file_upload_id: bool,
                    proof_document_file_upload_id: Option<String>,
    ignore_proof_document_file_upload_id: bool,
                ) -> Result<()>;
                async fn add_contact(
                    &self,
                    node_id: &bcr_ebill_core::NodeId,
                    t: ContactType,
                    name: Name,
                    email: Option<Email>,
                    postal_address: Option<PostalAddress>,
                    date_of_birth_or_registration: Option<Date>,
                    country_of_birth_or_registration: Option<Country>,
                    city_of_birth_or_registration: Option<City>,
                    identification_number: Option<Identification>,
                    avatar_file_upload_id: Option<String>,
                    proof_document_file_upload_id: Option<String>,
                ) -> Result<Contact>;
                async fn deanonymize_contact(
                    &self,
                    node_id: &bcr_ebill_core::NodeId,
                    t: ContactType,
                    name: Name,
                    email: Option<Email>,
                    postal_address: Option<PostalAddress>,
                    date_of_birth_or_registration: Option<Date>,
                    country_of_birth_or_registration: Option<Country>,
                    city_of_birth_or_registration: Option<City>,
                    identification_number: Option<Identification>,
                    avatar_file_upload_id: Option<String>,
                    proof_document_file_upload_id: Option<String>,
                ) -> Result<Contact>;
                async fn is_known_npub(&self, npub: &bcr_ebill_core::nostr_contact::NostrPublicKey) -> Result<bool>;
                async fn open_and_decrypt_file(
                    &self,
                    contact: Contact,
                    id: &bcr_ebill_core::NodeId,
                    file_name: &str,
                    private_key: &bcr_ebill_core::SecretKey,
                ) -> Result<Vec<u8>>;
                async fn get_nostr_npubs(&self) -> Result<Vec<bcr_ebill_core::nostr_contact::NostrPublicKey>>;
                async fn get_nostr_contact_by_node_id(&self, node_id: &bcr_ebill_core::NodeId) -> Result<Option<bcr_ebill_core::nostr_contact::NostrContact>>;
            }
        }

    mockall::mock! {
            pub IdentityServiceApi {}

            impl ServiceTraitBounds for IdentityServiceApi {}

            #[async_trait]
            impl IdentityServiceApi for IdentityServiceApi {
                async fn share_contact_details(&self, share_to: &bcr_ebill_core::NodeId) -> Result<()>;
                async fn publish_contact(&self, identity: &Identity, keys: &BcrKeys) -> Result<()>;
                async fn dev_mode_get_full_identity_chain(&self) -> Result<Vec<bcr_ebill_core::blockchain::identity::IdentityBlockPlaintextWrapper>>;
                async fn update_identity(
                    &self,
                    name: Option<Name>,
                    email: Option<Email>,
                    postal_address: OptionalPostalAddress,
                    date_of_birth: Option<Date>,
                    country_of_birth: Option<Country>,
                    city_of_birth: Option<City>,
                    identification_number: Option<Identification>,
                    profile_picture_file_upload_id: Option<String>,
    ignore_profile_picture_file_upload_id: bool,
                    identity_document_file_upload_id: Option<String>,
    ignore_identity_document_file_upload_id: bool,
                    timestamp: u64,
                ) -> Result<()>;
                async fn get_full_identity(&self) -> Result<IdentityWithAll>;
                async fn get_identity(&self) -> Result<Identity>;
                async fn identity_exists(&self) -> bool;
                async fn create_identity(
                    &self,
                    t: IdentityType,
                    name: Name,
                    email: Option<Email>,
                    postal_address: OptionalPostalAddress,
                    date_of_birth: Option<Date>,
                    country_of_birth: Option<Country>,
                    city_of_birth: Option<City>,
                    identification_number: Option<Identification>,
                    profile_picture_file_upload_id: Option<String>,
                    identity_document_file_upload_id: Option<String>,
                    timestamp: u64,
                ) -> Result<()>;
                async fn deanonymize_identity(
                    &self,
                    t: IdentityType,
                    name: Name,
                    email: Option<Email>,
                    postal_address: OptionalPostalAddress,
                    date_of_birth: Option<Date>,
                    country_of_birth: Option<Country>,
                    city_of_birth: Option<City>,
                    identification_number: Option<Identification>,
                    profile_picture_file_upload_id: Option<String>,
                    identity_document_file_upload_id: Option<String>,
                    timestamp: u64,
                ) -> Result<()>;
                async fn get_seedphrase(&self) -> Result<String>;
                async fn recover_from_seedphrase(&self, seed: &str) -> Result<()>;
                async fn open_and_decrypt_file(
                    &self,
                    identity: Identity,
                    id: &bcr_ebill_core::NodeId,
                    file_name: &str,
                    private_key: &bcr_ebill_core::SecretKey,
                ) -> Result<Vec<u8>>;
                async fn get_current_identity(&self) -> Result<ActiveIdentityState>;
                async fn set_current_personal_identity(&self, node_id: &bcr_ebill_core::NodeId) -> Result<()>;
                async fn set_current_company_identity(&self, node_id: &bcr_ebill_core::NodeId) -> Result<()>;
                async fn get_keys(&self) -> Result<BcrKeys>;
            }
        }
    mockall::mock! {
                        pub NotificationServiceApi {}

                        impl ServiceTraitBounds for NotificationServiceApi {}

                        #[async_trait]
                        impl NotificationServiceApi for NotificationServiceApi {
    async fn ensure_nostr_contact(&self, node_id: &bcr_ebill_core::NodeId);
        async fn connect(&self);
            async fn share_contact_details_keys(
                    &self,
                    recipient: &bcr_ebill_core::NodeId,
                    contact_id: &bcr_ebill_core::NodeId,
                    keys: &BcrKeys,
                ) -> NotifResult<()>;
                            async fn resync_identity_chain(&self) -> NotifResult<()>;
                            async fn resync_company_chain(&self, company_id: &bcr_ebill_core::NodeId) -> NotifResult<()>;
                            async fn publish_contact(&self, node_id: &bcr_ebill_core::NodeId, contact: &NostrContactData) -> NotifResult<()>;
                            async fn add_company_transport(&self, company: &bcr_ebill_core::company::Company, keys: &BcrKeys) -> NotifResult<()>;
                            async fn send_identity_chain_events(&self, events: IdentityChainEvent) -> NotifResult<()>;
                            async fn send_company_chain_events(&self, events: CompanyChainEvent) -> NotifResult<()>;
                            async fn resolve_contact(&self, node_id: &bcr_ebill_core::NodeId) -> NotifResult<Option<NostrContactData>>;
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
                                sender_node_id: &bcr_ebill_core::NodeId,
                                bill_id: &bcr_ebill_core::bill::BillId,
                                sum: Option<u64>,
                                timed_out_action: ActionType,
                                recipients: Vec<BillParticipant>,
                                holder: &bcr_ebill_core::NodeId,
                                drawee: &bcr_ebill_core::NodeId,
                                recoursee: &Option<bcr_ebill_core::NodeId>,
                            ) -> NotifResult<()>;
                            async fn send_recourse_action_event(
                                &self,
                                event: &BillChainEvent,
                                action: ActionType,
                                recoursee: &BillIdentParticipant,
                            ) -> NotifResult<()>;
                            async fn send_request_to_mint_event(
                                &self,
                                sender_node_id: &bcr_ebill_core::NodeId,
                                mint: &BillParticipant,
                                bill: &BitcreditBill,
                            ) -> NotifResult<()>;
                            async fn send_new_quote_event(&self, quote: &BitcreditBill) -> NotifResult<()>;
                            async fn send_quote_is_approved_event(&self, quote: &BitcreditBill) -> NotifResult<()>;
                            async fn get_client_notifications(
                                &self,
                                filter: NotificationFilter,
                            ) -> NotifResult<Vec<Notification>>;
                            async fn mark_notification_as_done(&self, notification_id: &str) -> NotifResult<()>;
                            async fn get_active_bill_notification(&self, bill_id: &bcr_ebill_core::bill::BillId) -> Option<Notification>;
                            async fn get_active_bill_notifications(
                                &self,
                                bill_ids: &[bcr_ebill_core::bill::BillId],
                            ) -> HashMap<bcr_ebill_core::bill::BillId, Notification>;
                            async fn check_bill_notification_sent(
                                &self,
                                bill_id: &bcr_ebill_core::bill::BillId,
                                block_height: i32,
                                action: ActionType,
                            ) -> NotifResult<bool>;
                            async fn mark_bill_notification_sent(
                                &self,
                                bill_id: &bcr_ebill_core::bill::BillId,
                                block_height: i32,
                                action: ActionType,
                            ) -> NotifResult<()>;
                            async fn send_retry_messages(&self) -> NotifResult<()>;
                async fn get_active_notification_status_for_node_ids(
                        &self,
                        node_ids: &[bcr_ebill_core::NodeId],
                    ) -> NotifResult<HashMap<bcr_ebill_core::NodeId, bool>>;
                async fn register_email_notifications(
                        &self,
                        relay_url: &url::Url,
                        email: &Email,
                        node_id: &bcr_ebill_core::NodeId,
                        caller_keys: &BcrKeys,
                    ) -> NotifResult<()>;
                async fn get_email_notifications_preferences_link(&self, node_id: &bcr_ebill_core::NodeId) -> NotifResult<url::Url>;
                async fn resync_bill_chain(&self, bill_id: &bcr_ebill_core::bill::BillId) -> NotifResult<()>;
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
