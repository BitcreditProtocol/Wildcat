// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{get, post, put},
    Router,
};
use bcr_common::wire::{
    bill as wire_bill, clowder as wire_clowder, identity as wire_identity, keys as wire_keys,
    quotes as wire_quotes,
};
use utoipa::OpenApi;
// ----- local modules
mod admin;
mod error;

// ----- end imports

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    pub keys_url: bcr_common::client::Url,
    pub quotes_url: bcr_common::client::Url,
    pub ebill_url: bcr_common::client::Url,
    pub clowder_url: bcr_common::client::Url,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    pub keys_cl: bcr_common::client::keys::Client,
    pub quotes_cl: bcr_common::client::quote::Client,
    pub ebill_cl: bcr_common::client::ebill::Client,
    pub clwdr_cl: Arc<clwdr_client::ClowderRestClient>,
}

impl AppController {
    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig {
            keys_url,
            quotes_url,
            ebill_url,
            clowder_url,
        } = cfg;
        let keys_cl = bcr_common::client::keys::Client::new(keys_url);
        let quotes_cl = bcr_common::client::quote::Client::new(quotes_url);
        let ebill_cl = bcr_common::client::ebill::Client::new(ebill_url);
        let clwdr_cl = clwdr_client::ClowderRestClient::new(clowder_url);
        AppController {
            keys_cl,
            quotes_cl,
            ebill_cl,
            clwdr_cl: Arc::new(clwdr_cl),
        }
    }
}

pub mod endpoints {
    pub const HEALTH: &str = "/health";
    // Keys-Client
    pub const KEYSET_INFO: &str = "/v1/admin/keysets/{kid}";
    pub const LIST_KEYSET_INFOS: &str = "/v1/admin/keysets";
    pub const ENABLE_REDEMPTION: &str = "/v1/admin/credit/enable_redemption";
    pub const MINT_OP_STATUS: &str = "/v1/admin/credit/mint_op_status/{qid}";
    pub const LIST_MINT_OPS: &str = "/v1/admin/credit/mint_ops/{kid}";
    // Quotes-Client
    pub const GET_CREDIT_QUOTE: &str = "/v1/admin/credit/quote/{qid}";
    pub const LIST_CREDIT_QUOTES: &str = "/v1/admin/credit/quote";
    pub const UPDATE_CREDIT_QUOTE: &str = "/v1/admin/credit/quote/{qid}";
    pub const ENABLE_QUOTE_MINTING: &str = "/v1/admin/credit/quote/enable_mint/{qid}";
    // EBills-Client
    pub const GET_IDENTITY: &str = "/v1/admin/ebill/identity";
    pub const GET_EBILL: &str = "/v1/admin/ebill/bills/{bid}";
    pub const LIST_EBILLS: &str = "/v1/admin/ebill/bills";
    pub const GET_EBILL_ENDORSEMENTS: &str = "/v1/admin/ebill/endorsements/{bid}";
    pub const GET_EBILL_ATTACHMENT: &str = "/v1/admin/ebill/attachments/{bid}/{fname}";
    pub const POST_EBILL_REQTOPAY: &str = "/v1/admin/ebill/reqtopay";
    // Clowder-Client
    pub const GET_CLOWDER_ALPHAS: &str = "/v1/admin/clowder/alphas";
    pub const GET_CLOWDER_BETAS: &str = "/v1/admin/clowder/betas";
    pub const GET_CLOWDER_MYSTATUS: &str = "/v1/admin/clowder/status";
    pub const GET_CLOWDER_STATUS: &str = "/v1/admin/clowder/status/{pk}";
}

pub fn routes(ctrl: AppController) -> Router {
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());
    let admin = Router::new()
        .route(endpoints::HEALTH, get(admin::get_health))
        // keys service
        .route(endpoints::KEYSET_INFO, get(admin::get_keyset_info))
        .route(endpoints::LIST_KEYSET_INFOS, get(admin::list_keyset_infos))
        .route(endpoints::MINT_OP_STATUS, get(admin::get_mintop_status))
        .route(endpoints::LIST_MINT_OPS, get(admin::list_mintops))
        .route(
            endpoints::ENABLE_REDEMPTION,
            post(admin::post_enable_redemption),
        )
        // quotes service
        .route(endpoints::GET_CREDIT_QUOTE, get(admin::get_quote))
        .route(endpoints::LIST_CREDIT_QUOTES, get(admin::list_quotes))
        .route(endpoints::UPDATE_CREDIT_QUOTE, put(admin::update_quote))
        .route(
            endpoints::ENABLE_QUOTE_MINTING,
            post(admin::post_enable_quote_minting),
        )
        // ebills service
        .route(endpoints::GET_IDENTITY, get(admin::get_identity))
        .route(endpoints::GET_EBILL, get(admin::get_ebill))
        .route(endpoints::LIST_EBILLS, get(admin::list_ebills))
        .route(
            endpoints::GET_EBILL_ENDORSEMENTS,
            get(admin::get_ebill_endorsements),
        )
        .route(
            endpoints::GET_EBILL_ATTACHMENT,
            get(admin::get_ebill_attachment),
        )
        .route(
            endpoints::POST_EBILL_REQTOPAY,
            post(admin::post_ebill_reqtopay),
        )
        // clowder service
        .route(
            endpoints::GET_CLOWDER_ALPHAS,
            get(admin::get_clowder_alphas),
        )
        .route(endpoints::GET_CLOWDER_BETAS, get(admin::get_clowder_betas))
        .route(
            endpoints::GET_CLOWDER_MYSTATUS,
            get(admin::get_clowder_mystatus),
        )
        .route(
            endpoints::GET_CLOWDER_STATUS,
            get(admin::get_clowder_status),
        );
    Router::new().merge(admin).with_state(ctrl).merge(swagger)
}

#[derive(utoipa::OpenApi)]
#[openapi(
    components(schemas(
        // keys service
        cashu::Id,
        cashu::KeySetInfo,
        wire_keys::MintOperationStatus,
        wire_keys::DeactivateKeysetRequest,
        wire_keys::DeactivateKeysetResponse,
        // quotes service
        wire_quotes::StatusReply,
        wire_quotes::ListReplyLight,
        wire_quotes::UpdateQuoteRequest,
        wire_quotes::UpdateQuoteResponse,
        wire_quotes::EnableMintingResponse,
        // ebills service
        wire_identity::Identity,
        wire_bill::BitcreditBill,
        wire_bill::Endorsement,
        wire_bill::RequestToPayBitcreditBillPayload,
        // clowder service
        wire_clowder::ConnectedMintsResponse,
        wire_clowder::PerceivedState,
        wire_clowder::AlphaStateResponse,
    ),),
    paths(
        admin::get_health,
        // keys service
        admin::get_keyset_info,
        admin::list_keyset_infos,
        admin::get_mintop_status,
        admin::list_mintops,
        admin::post_enable_redemption,
        // quotes service
        admin::get_quote,
        admin::list_quotes,
        admin::update_quote,
        admin::post_enable_quote_minting,
        // ebills service
        admin::get_identity,
        admin::get_ebill,
        admin::list_ebills,
        admin::get_ebill_endorsements,
        admin::get_ebill_attachment,
        admin::post_ebill_reqtopay,
        // clowder service
        admin::get_clowder_alphas,
        admin::get_clowder_betas,
        admin::get_clowder_mystatus,
        admin::get_clowder_status,
    )
)]
pub struct ApiDoc;
impl ApiDoc {
    pub fn generate_yml() -> Option<String> {
        ApiDoc::openapi().to_yaml().ok()
    }
    pub fn generate_json() -> Option<String> {
        ApiDoc::openapi().to_pretty_json().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_should_successfully_generate_openapi_docs() {
        let yml = ApiDoc::generate_yml();
        assert!(yml.is_some());

        let json = ApiDoc::generate_json();
        assert!(json.is_some());
    }
}
