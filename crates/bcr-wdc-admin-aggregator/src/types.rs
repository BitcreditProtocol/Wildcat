// ----- standard library imports
// ----- extra library imports
// ----- local imports

// ----- end imports

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct TokenStateRequest {
    pub token: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum TokenState {
    Unspent,
    Spent,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct TokenStateResponse {
    pub state: TokenState,
}
