// ----- standard library imports
// ----- extra library imports
use axum::{extract::State, Json};
use log::{error, info};
// ----- local imports
use crate::{error::Result, AppController};
// ----- end imports

#[derive(Debug, Clone, serde::Serialize)]
pub struct TestResponse {
    ok: bool,
}

pub async fn test(State(ctrl): State<AppController>) -> Result<Json<TestResponse>> {
    match ctrl.identity_service.identity_exists().await {
        true => {
            info!("Identity exists");
        }
        false => {
            error!("No Identity found");
        }
    };
    Ok(Json(TestResponse { ok: true }))
}
