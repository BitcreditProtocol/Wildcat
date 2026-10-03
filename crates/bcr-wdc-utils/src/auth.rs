// ----- standard library imports
use std::sync::Arc;
// ----- extra library imports
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
// ----- end imports

/// Compares two byte strings in constant time, so a timing side-channel cannot be
/// used to guess the secret one character at a time.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

async fn check_bearer_token(
    State(secret): State<Arc<str>>,
    req: Request,
    next: Next,
) -> Response {
    let presented = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));

    match presented {
        Some(token) if constant_time_eq(token.as_bytes(), secret.as_bytes()) => {
            next.run(req).await
        }
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// Wraps `router` so every request must carry `Authorization: Bearer <secret>`,
/// answering `401 Unauthorized` otherwise.
pub fn require_api_key<S>(router: Router<S>, secret: impl Into<String>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let secret: Arc<str> = Arc::from(secret.into());
    router.layer(middleware::from_fn_with_state(secret, check_bearer_token))
}
