// ----- standard library imports
// ----- extra library imports
use axum::extract::FromRef;
use axum::routing::get;
use axum::Router;
use utoipa::OpenApi;
// ----- local modules
mod admin;
mod error;

// ----- end imports

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct AppConfig {}

#[derive(Clone, FromRef)]
pub struct AppController {}

impl AppController {
    pub async fn new(_cfg: AppConfig) -> Self {
        Self {}
    }
}

pub fn routes(ctrl: AppController) -> Router {
    let swagger = utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
        .url("/api-docs/openapi.json", ApiDoc::openapi());

    let admin = Router::new().route("/v1/admin/eiou/balance", get(admin::balance));

    Router::new().merge(admin).with_state(ctrl).merge(swagger)
}

#[derive(utoipa::OpenApi)]
#[openapi(
    components(schemas(bcr_wdc_webapi::eiou::BalanceResponse,),),
    paths(admin::balance,)
)]
struct ApiDoc;
