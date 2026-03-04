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

#[derive(Debug, Default, serde::Deserialize, utoipa::IntoParams)]
pub struct KeysetListParam {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}
