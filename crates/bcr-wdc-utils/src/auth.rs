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

/// RFC 7235 2.1: `credentials = auth-scheme [ 1*SP token68 ]`, and the scheme is
/// case-insensitive. Splits on the first run of whitespace between scheme and
/// token68, rather than requiring exactly one literal space, and trims outer
/// whitespace a transport may not have (e.g. an inner tab).
fn parse_bearer(value: &str) -> Option<&str> {
    let value = value.trim();
    let mut parts = value.splitn(2, char::is_whitespace);
    let scheme = parts.next()?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    Some(parts.next().unwrap_or("").trim())
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
        .and_then(parse_bearer);

    match presented {
        Some(token) if constant_time_eq(token.as_bytes(), secret.as_bytes()) => {
            next.run(req).await
        }
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// Wraps `router` so every request must carry `Authorization: Bearer <secret>`,
/// answering `401 Unauthorized` otherwise.
///
/// Panics if `secret` is empty, carries a byte outside the HTTP header
/// `field-vchar` set (RFC 7230 3.2: visible ASCII plus internal space/tab,
/// no control characters, no leading/trailing whitespace): a secret shaped
/// like that could never be presented by its own correct caller — a
/// transport that accepts it at all strips or rejects the offending bytes —
/// and an empty one would grant access to anyone presenting `Bearer ` with
/// no token.
pub fn require_api_key<S>(router: Router<S>, secret: impl Into<String>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let secret = secret.into();
    assert!(!secret.is_empty(), "require_api_key: secret must not be empty");
    assert!(secret.is_ascii(), "require_api_key: secret must be ASCII");
    assert_eq!(
        secret,
        secret.trim(),
        "require_api_key: secret must not carry leading/trailing whitespace"
    );
    assert!(
        secret.bytes().all(|b| b == b' ' || b == b'\t' || (0x21..=0x7e).contains(&b)),
        "require_api_key: secret must contain only visible ASCII and internal space/tab, no control characters"
    );
    let secret: Arc<str> = Arc::from(secret);
    router.layer(middleware::from_fn_with_state(secret, check_bearer_token))
}
