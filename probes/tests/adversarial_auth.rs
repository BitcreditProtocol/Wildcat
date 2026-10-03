//! Adversarial probes of `bcr_wdc_utils::auth::require_api_key` over real TCP.
mod adversarial_common;

use std::net::SocketAddr;

use adversarial_common::raw;
use axum::{routing::get, routing::post, Router};
use bcr_wdc_utils::auth::require_api_key;

const SECRET: &str = "s3cr3t-T0ken";

fn guarded(secret: &str) -> Router {
    let inner = Router::new()
        .route("/admin/x", post(|| async { "REACHED" }))
        .route("/admin/y", get(|| async { "REACHED" }));
    require_api_key(inner, secret.to_string())
}

async fn serve(router: Router) -> SocketAddr {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, router).await.unwrap() });
    addr
}

fn req_with(path: &str, headers: &[&[u8]]) -> Vec<u8> {
    let mut r = format!("POST {path} HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nConnection: close\r\n")
        .into_bytes();
    for h in headers {
        r.extend_from_slice(h);
        r.extend_from_slice(b"\r\n");
    }
    r.extend_from_slice(b"\r\n");
    r
}

async fn status(addr: SocketAddr, headers: &[&[u8]]) -> (u16, bool) {
    let (s, body) = raw(addr, &req_with("/admin/x", headers)).await;
    (s, body.contains("REACHED"))
}

/// An auth layer configured with an empty secret must not let anyone in: either
/// construction refuses it, or every request is rejected.
#[tokio::test]
async fn empty_secret_never_grants_access_over_tcp() {
    let built = std::panic::catch_unwind(|| guarded(""));
    let Ok(router) = built else { return };
    let addr = serve(router).await;
    let mut reached = vec![];
    for h in [
        &b""[..],
        b"Authorization: Bearer ",
        b"Authorization: Bearer",
        b"Authorization: Bearer \t",
        b"Authorization:Bearer ",
    ] {
        let hs: Vec<&[u8]> = if h.is_empty() { vec![] } else { vec![h] };
        let (s, r) = status(addr, &hs).await;
        println!("empty secret, header {:?} -> {s}", String::from_utf8_lossy(h));
        if r {
            reached.push(String::from_utf8_lossy(h).to_string());
        }
    }
    let h2 = reqwest::Client::builder()
        .http2_prior_knowledge()
        .build()
        .unwrap()
        .post(format!("http://{addr}/admin/x"))
        .header("authorization", "Bearer ")
        .send()
        .await;
    match h2 {
        Ok(resp) => {
            let s = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            println!("empty secret, h2 prior knowledge `Bearer ` -> {s}");
            if body.contains("REACHED") {
                reached.push("h2: Authorization: Bearer ".into());
            }
        }
        Err(e) => println!("h2 prior knowledge not served: {e}"),
    }
    assert!(reached.is_empty(), "empty secret let these through: {reached:?}");
}

/// Same as above, but where the transport does not trim the header value
/// (in-process / any non-trimming hop): `Bearer ` with an empty secret.
#[tokio::test]
async fn empty_secret_never_grants_access_untrimmed_value() {
    let built = std::panic::catch_unwind(|| guarded(""));
    let Ok(router) = built else { return };
    let server = axum_test::TestServer::new(router).unwrap();
    let resp = server
        .post("/admin/x")
        .add_header("authorization", "Bearer ")
        .await;
    assert_eq!(
        resp.status_code().as_u16(),
        401,
        "require_api_key(\"\") accepted `Authorization: Bearer ` -> body {:?}",
        resp.text()
    );
}

#[tokio::test]
async fn wrong_and_malformed_tokens_are_rejected() {
    let addr = serve(guarded(SECRET)).await;
    let short = &SECRET[..SECRET.len() - 1];
    let cases: Vec<Vec<u8>> = vec![
        vec![],
        b"Authorization: Bearer".to_vec(),
        b"Authorization: Bearer ".to_vec(),
        format!("Authorization: Bearer {SECRET}X").into_bytes(),
        format!("Authorization: Bearer {short}").into_bytes(),
        format!("Authorization: Bearer {SECRET} extra").into_bytes(),
        format!("Authorization: Bearer {SECRET},Bearer x").into_bytes(),
        format!("Authorization: Basic {SECRET}").into_bytes(),
        format!("Authorization: {SECRET}").into_bytes(),
        format!("Authorization: Bearer\t{SECRET}").into_bytes(),
        format!("X-Authorization: Bearer {SECRET}").into_bytes(),
        format!("Proxy-Authorization: Bearer {SECRET}").into_bytes(),
        [b"Authorization: Bearer \xff\xfe".as_slice(), SECRET.as_bytes()].concat(),
        [b"Authorization: Bearer ".as_slice(), SECRET.as_bytes(), b"\x80"].concat(),
    ];
    let mut reached = vec![];
    for c in &cases {
        let hs: Vec<&[u8]> = if c.is_empty() { vec![] } else { vec![c] };
        let (s, r) = status(addr, &hs).await;
        println!("{:?} -> {s}", String::from_utf8_lossy(c));
        if r || s == 200 {
            reached.push(String::from_utf8_lossy(c).to_string());
        }
    }
    assert!(reached.is_empty(), "accepted: {reached:?}");
}

#[tokio::test]
async fn duplicate_authorization_headers() {
    let addr = serve(guarded(SECRET)).await;
    let good = format!("Authorization: Bearer {SECRET}");
    let bad = b"Authorization: Bearer nope".as_slice();
    let (s1, r1) = status(addr, &[bad, good.as_bytes()]).await;
    let (s2, r2) = status(addr, &[good.as_bytes(), bad]).await;
    println!("wrong,right -> {s1} reached={r1}; right,wrong -> {s2} reached={r2}");
    assert!(!r1, "a wrong first credential followed by a right one was accepted");
    // Documented, not a security defect (the sender already holds the secret):
    // the first header wins, a conflicting second one is ignored rather than refused.
    assert!(r2, "right,wrong behaviour changed: {s2}");
}

#[tokio::test]
async fn one_megabyte_token_is_rejected_and_server_survives() {
    let addr = serve(guarded(SECRET)).await;
    let huge = format!("Authorization: Bearer {}", "A".repeat(1 << 20));
    let (s, r) = status(addr, &[huge.as_bytes()]).await;
    println!("1MB token -> {s}");
    assert!(!r, "1MB token reached the handler");
    let huge_prefixed = format!("Authorization: Bearer {SECRET}{}", "A".repeat(1 << 20));
    let (s, r) = status(addr, &[huge_prefixed.as_bytes()]).await;
    println!("secret + 1MB -> {s}");
    assert!(!r, "secret-prefixed 1MB token reached the handler");
    let good = format!("Authorization: Bearer {SECRET}");
    let (s, r) = status(addr, &[good.as_bytes()]).await;
    assert!(r && s == 200, "server did not serve a valid request after the 1MB tokens: {s}");
}

/// Unauthenticated callers should not be able to tell existing admin routes from
/// missing ones (404 / 405 before 401 is a route-enumeration oracle).
#[tokio::test]
async fn unauthenticated_route_enumeration_oracle() {
    let addr = serve(guarded(SECRET)).await;
    let (exists, _) = raw(addr, &req_with("/admin/x", &[])).await;
    let (missing, _) = raw(addr, &req_with("/admin/does-not-exist", &[])).await;
    let (wrong_method, _) = raw(
        addr,
        b"DELETE /admin/x HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    )
    .await;
    println!("existing={exists} missing={missing} wrong_method={wrong_method}");
    assert_eq!(
        (exists, missing, wrong_method),
        (401, 401, 401),
        "unauthenticated responses differ by route"
    );
}

/// RFC 7235 2.1: the auth-scheme is case-insensitive. A correct token must not be
/// refused because of the scheme's case.
#[tokio::test]
async fn interop_scheme_is_case_insensitive() {
    let addr = serve(guarded(SECRET)).await;
    let mut refused = vec![];
    for scheme in ["Bearer", "bearer", "BEARER", "bEaReR"] {
        let h = format!("Authorization: {scheme} {SECRET}");
        let (s, r) = status(addr, &[h.as_bytes()]).await;
        println!("{scheme} -> {s}");
        if !r {
            refused.push(format!("{scheme} -> {s}"));
        }
    }
    assert!(refused.is_empty(), "correct token refused: {refused:?}");
}

/// RFC 7235: `credentials = auth-scheme [ 1*SP token68 ]`; header OWS is not part
/// of the value. A correct token with legal extra whitespace must be accepted.
#[tokio::test]
async fn interop_legal_whitespace() {
    let addr = serve(guarded(SECRET)).await;
    let mut refused = vec![];
    for h in [
        format!("Authorization: Bearer {SECRET}"),
        format!("Authorization: Bearer {SECRET} "),
        format!("Authorization:   Bearer {SECRET}"),
        format!("Authorization: Bearer {SECRET}\t"),
        format!("Authorization: Bearer  {SECRET}"),
    ] {
        let (s, r) = status(addr, &[h.as_bytes()]).await;
        println!("{h:?} -> {s}");
        if !r {
            refused.push(format!("{h:?} -> {s}"));
        }
    }
    assert!(refused.is_empty(), "correct token refused: {refused:?}");
}

/// A secret that can never be presented successfully must be refused when the
/// layer is built, not silently lock every caller out.
#[tokio::test]
async fn unpresentable_secret_is_refused_or_works() {
    let mut locked_out = vec![];
    for secret in ["s\u{e9}cret", "trailing-space ", " leading-space"] {
        let built = std::panic::catch_unwind(|| guarded(secret));
        let Ok(router) = built else { continue };
        let addr = serve(router).await;
        let h = [b"Authorization: Bearer ".as_slice(), secret.as_bytes()].concat();
        let (s, r) = status(addr, &[&h]).await;
        println!("secret {secret:?}, presented verbatim -> {s}");
        if !r {
            locked_out.push(format!("{secret:?} -> {s}"));
        }
    }
    assert!(
        locked_out.is_empty(),
        "require_api_key accepted secrets the correct caller can never present: {locked_out:?}"
    );
}
