// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::extract::{Json, State};
use bcr_common::wire::wallet::Network;
// ----- local imports
use crate::error::Result;
use crate::service::Service;

// ----- end imports

/// --------------------------- Look up keysets info
#[tracing::instrument(level = tracing::Level::DEBUG, skip(ctrl))]
pub async fn network(State(ctrl): State<Arc<Service>>) -> Result<Json<Network>> {
    tracing::debug!("Received network request");

    let net = ctrl.network();
    Ok(Json(Network { network: net }))
}
