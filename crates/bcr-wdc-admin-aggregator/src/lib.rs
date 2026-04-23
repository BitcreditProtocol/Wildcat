// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::FromRef,
    routing::{get, post, put},
    Router,
};
use bcr_common::{
    cashu,
    client::{
        admin::clowder::Client as ClowderClient, core::Client as CoreClient,
        ebill::Client as EbillClient, quote::Client as QuoteClient,
        treasury::Client as TreasuryClient, Url as ClientUrl,
    },
    wire::{
        bill as wire_bill, clowder as wire_clowder, common as wire_common,
        identity as wire_identity, info as wire_info, keys as wire_keys, quotes as wire_quotes,
        treasury as wire_treasury, wallet as wire_wallet,
    },
};
use utoipa::OpenApi;
// ----- local modules
mod admin;
mod error;
mod types;

// ----- end imports

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AppConfig {
    pub core_url: ClientUrl,
    pub quotes_url: ClientUrl,
    pub ebill_url: ClientUrl,
    pub clowder_url: ClientUrl,
    pub treasury_url: ClientUrl,
}

#[derive(Clone, FromRef)]
pub struct AppController {
    pub core_cl: CoreClient,
    pub quotes_cl: QuoteClient,
    pub ebill_cl: EbillClient,
    pub clwdr_cl: Arc<ClowderClient>,
    pub treasury_cl: TreasuryClient,
}

impl AppController {
    const MINIMUM_KEYSET_FEE_RATE_PPK: u64 = 0;

    pub async fn new(cfg: AppConfig) -> Self {
        let AppConfig {
            core_url,
            quotes_url,
            ebill_url,
            clowder_url,
            treasury_url,
        } = cfg;
        let core_cl = CoreClient::new(core_url);
        let quotes_cl = QuoteClient::new(quotes_url);
        let ebill_cl = EbillClient::new(ebill_url);
        let clwdr_cl = ClowderClient::new(clowder_url);
        let treasury_cl = TreasuryClient::new(treasury_url);

        // pre-flight checklist
        let filter = wire_keys::KeysetInfoFilters {
            unit: Some(CoreClient::currency_unit()),
            ..Default::default()
        };
        let kinfos = core_cl
            .list_keyset_info(filter)
            .await
            .expect("pre-flight check failed: core service is not responding");
        let perpetual_kinfo = kinfos.iter().find(|k| k.final_expiry.is_none() && k.active);
        if perpetual_kinfo.is_none() {
            tracing::warn!(
                "pre-flight check warning: core service has no perpetual keysets configured"
            );
            core_cl
                .new_keyset(None, Self::MINIMUM_KEYSET_FEE_RATE_PPK)
                .await
                .expect("pre-flight check failed: core service is not responding to new_keyset");
        }
        AppController {
            core_cl,
            quotes_cl,
            ebill_cl,
            clwdr_cl: Arc::new(clwdr_cl),
            treasury_cl,
        }
    }
}

pub mod endpoints {
    pub const HEALTH: &str = "/health";
    pub const MINT_INFO: &str = "/v1/admin/mint/info";
    // Core-Client
    pub const KEYSET_INFO: &str = "/v1/admin/keysets/{kid}";
    pub const LIST_KEYSET_INFOS: &str = "/v1/admin/keysets";
    pub const ENABLE_REDEMPTION: &str = "/v1/admin/credit/enable_redemption";
    pub const POST_TOKEN_STATUS: &str = "/v1/admin/credit/token_status";
    // Quotes-Client
    pub const GET_CREDIT_QUOTE: &str = "/v1/admin/credit/quote/{qid}";
    pub const LIST_CREDIT_QUOTES: &str = "/v1/admin/credit/quote";
    pub const UPDATE_CREDIT_QUOTE: &str = "/v1/admin/credit/quote/{qid}";
    // EBills-Client
    pub const GET_IDENTITY: &str = "/v1/admin/ebill/identity";
    pub const GET_EBILL: &str = "/v1/admin/ebill/bills/{bid}";
    pub const LIST_EBILLS: &str = "/v1/admin/ebill/bills";
    pub const GET_EBILL_ENDORSEMENTS: &str = "/v1/admin/ebill/endorsements/{bid}";
    pub const GET_EBILL_ATTACHMENT: &str = "/v1/admin/ebill/attachments/{bid}/{fname}";
    pub const GET_EBILL_FILE_FROM_REQUEST_TO_MINT: &str =
        "/v1/admin/ebill/get_file_from_request_to_mint";
    pub const GET_EBILL_PAYMENTSTATUS: &str = "/v1/admin/ebill/payment_status/{bid}";
    // Clowder-Client
    pub const GET_CLOWDER_INFO: &str = "/v1/admin/clowder/info";
    pub const GET_CLOWDER_ALPHAS: &str = "/v1/admin/clowder/alphas";
    pub const GET_CLOWDER_BETAS: &str = "/v1/admin/clowder/betas";
    pub const GET_CLOWDER_LOCAL_COVERAGE: &str = "/v1/admin/clowder/coverage";
    pub const GET_CLOWDER_FOREIGN_COVERAGE: &str = "/v1/admin/clowder/coverage/{pk}";
    pub const GET_CLOWDER_MYSTATUS: &str = "/v1/admin/clowder/status";
    pub const GET_CLOWDER_STATUS: &str = "/v1/admin/clowder/status/{pk}";
    // Treasury-Client
    pub const MINT_OP_STATUS: &str = "/v1/admin/treasury/credit/mint_op_status/{qid}";
    pub const LIST_MINT_OPS: &str = "/v1/admin/treasury/credit/mint_ops/{kid}";
    pub const POST_EBILL_REQTOPAY: &str = "/v1/admin/treasury/ebill/reqtopay";
}

pub fn routes(ctrl: AppController) -> Router {
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());
    let admin = Router::new()
        .route(endpoints::HEALTH, get(admin::get_health))
        .route(endpoints::MINT_INFO, get(admin::get_mint_info))
        // core service
        .route(endpoints::KEYSET_INFO, get(admin::get_keyset_info))
        .route(endpoints::LIST_KEYSET_INFOS, get(admin::list_keyset_infos))
        .route(
            endpoints::ENABLE_REDEMPTION,
            post(admin::post_enable_redemption),
        )
        .route(endpoints::POST_TOKEN_STATUS, post(admin::post_token_status))
        // quotes service
        .route(endpoints::GET_CREDIT_QUOTE, get(admin::get_quote))
        .route(endpoints::LIST_CREDIT_QUOTES, get(admin::list_quotes))
        .route(endpoints::UPDATE_CREDIT_QUOTE, put(admin::update_quote))
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
            endpoints::GET_EBILL_FILE_FROM_REQUEST_TO_MINT,
            get(admin::get_ebill_file_from_request_to_mint),
        )
        .route(
            endpoints::GET_EBILL_PAYMENTSTATUS,
            get(admin::get_ebill_paymentstatus),
        )
        // clowder service
        .route(endpoints::GET_CLOWDER_INFO, get(admin::get_clowder_info))
        .route(
            endpoints::GET_CLOWDER_ALPHAS,
            get(admin::get_clowder_alphas),
        )
        .route(endpoints::GET_CLOWDER_BETAS, get(admin::get_clowder_betas))
        .route(
            endpoints::GET_CLOWDER_LOCAL_COVERAGE,
            get(admin::get_clowder_local_coverage),
        )
        .route(
            endpoints::GET_CLOWDER_FOREIGN_COVERAGE,
            get(admin::get_clowder_foreign_coverage),
        )
        .route(
            endpoints::GET_CLOWDER_MYSTATUS,
            get(admin::get_clowder_mystatus),
        )
        .route(
            endpoints::GET_CLOWDER_STATUS,
            get(admin::get_clowder_status),
        )
        // treasury service
        .route(endpoints::MINT_OP_STATUS, get(admin::get_mintop_status))
        .route(endpoints::LIST_MINT_OPS, get(admin::list_mintops))
        .route(
            endpoints::POST_EBILL_REQTOPAY,
            post(admin::post_ebill_reqtopay),
        );
    Router::new().merge(admin).with_state(ctrl).merge(swagger)
}

#[derive(utoipa::OpenApi)]
#[openapi(
    components(schemas(
        wire_info::WildcatInfo,
        // common
        wire_common::PaginatedResponse<cashu::KeySetInfo>,
        wire_common::PaginatedResponse<wire_quotes::LightInfo>,
        // core service
        cashu::Id,
        cashu::KeySetInfo,
        wire_keys::DeactivateKeysetRequest,
        wire_keys::DeactivateKeysetResponse,
        types::TokenStateRequest,
        types::TokenStateResponse,
        // quotes service
        wire_quotes::ListSort,
        wire_quotes::InfoReply,
        wire_quotes::LightInfo,
        wire_quotes::UpdateQuoteRequest,
        wire_quotes::UpdateQuoteResponse,
        // ebills service
        wire_identity::Identity,
        wire_bill::BitcreditBill,
        wire_bill::Endorsement,
        wire_bill::SimplifiedBillPaymentStatus,
        // clowder service
        wire_clowder::ClowderNodeInfo,
        wire_clowder::ConnectedMintsResponse,
        wire_clowder::PerceivedState,
        wire_clowder::AlphaStateResponse,
        wire_clowder::Coverage,
        // treasury service
        wire_treasury::RequestToPayFromEBillRequest,
        wire_treasury::RequestToPayFromEBillResponse,
        wire_treasury::MintOperationStatus,
        wire_wallet::ECashBalance,
        wire_wallet::EbillPaymentComplete,
    ),),
    paths(
        admin::get_health,
        admin::get_mint_info,
        // core service
        admin::get_keyset_info,
        admin::list_keyset_infos,
        admin::get_mintop_status,
        admin::list_mintops,
        admin::post_enable_redemption,
        admin::post_token_status,
        // quotes service
        admin::get_quote,
        admin::list_quotes,
        admin::update_quote,
        // ebills service
        admin::get_identity,
        admin::get_ebill,
        admin::list_ebills,
        admin::get_ebill_endorsements,
        admin::get_ebill_attachment,
        admin::get_ebill_paymentstatus,
        admin::get_ebill_file_from_request_to_mint,
        // clowder service
        admin::get_clowder_info,
        admin::get_clowder_alphas,
        admin::get_clowder_betas,
        admin::get_clowder_local_coverage,
        admin::get_clowder_foreign_coverage,
        admin::get_clowder_mystatus,
        admin::get_clowder_status,
        // treasury service
        admin::post_ebill_reqtopay,
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
