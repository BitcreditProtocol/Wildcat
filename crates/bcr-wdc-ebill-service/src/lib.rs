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
        notification_service::{
            create_nostr_clients, create_nostr_consumer, create_notification_service, NostrConsumer,
        },
    },
};
use bcr_ebill_transport::{NotificationServiceApi, PushApi, PushService};
// ----- local modules
mod bill;
mod contact;
mod error;
mod identity;
mod web;
// ----- end imports

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    pub ebill_db: ConnectionConfig,
    pub bitcoin_network: String,
    pub esplora_base_url: String,
    pub nostr_relays: Vec<String>,
    pub data_dir: String,
    pub job_runner_initial_delay_seconds: u64,
    pub job_runner_check_interval_seconds: u64,
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
    pub nostr_consumer: NostrConsumer,
    pub notification_service: Arc<dyn NotificationServiceApi>,
    pub push_service: Arc<dyn PushApi>,
}

impl AppController {
    pub async fn new(cfg: bcr_ebill_api::Config, db: bcr_ebill_api::DbContext) -> Self {
        let contact_service = Arc::new(ContactService::new(
            db.contact_store.clone(),
            db.file_upload_store.clone(),
            db.identity_store.clone(),
        ));

        let nostr_clients =
            create_nostr_clients(&cfg, db.identity_store.clone(), db.company_store.clone())
                .await
                .expect("Failed to create nostr clients");
        let notification_service = create_notification_service(
            nostr_clients.clone(),
            db.notification_store.clone(),
            contact_service.clone(),
            db.queued_message_store.clone(),
            cfg.nostr_relays.clone(),
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

        let nostr_consumer = create_nostr_consumer(
            nostr_clients,
            contact_service.clone(),
            db.nostr_event_offset_store.clone(),
            db.notification_store.clone(),
            push_service.clone(),
            db.bill_blockchain_store.clone(),
            db.bill_store.clone(),
        )
        .await
        .expect("Failed to create Nostr consumer");

        Self {
            contact_service,
            bill_service,
            identity_service: Arc::new(identity_service),
            nostr_consumer,
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
            "/bill/attachment/{bill_id}/{file_name}",
            get(web::get_bill_attachment),
        )
        .route("/bill/request_to_pay", put(web::request_to_pay_bill))
        .route("/bill/bitcoin_key/{bill_id}", get(web::bill_bitcoin_key))
        .with_state(ctrl)
}
