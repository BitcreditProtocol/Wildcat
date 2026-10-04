//! BINARY lens: the real `bcr-wdc-admin-aggregator` executable (built by `probes/run`
//! before the tests), configured the way a deployer does it (config.toml in its cwd,
//! env `ADMIN_AGGREGATOR_*`), against in-process core/quote/treasury splits.

mod integration_common;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use bcr_wdc_admin_aggregator::endpoints as ep;
use integration_common::*;

fn bin() -> PathBuf {
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target"));
    let p = target.join("debug/bcr-wdc-admin-aggregator");
    assert!(p.exists(), "{p:?} missing: run `bash probes/run`, which builds it first");
    p.canonicalize().unwrap()
}

struct Backends {
    log: Log,
    core: Split,
    quote: Split,
    treasury: Split,
    unused: bcr_common::client::Url,
}

async fn backends() -> Backends {
    let log = Log::default();
    let (core, _) = core_split(&log).await;
    let quote = quote_split(&log).await;
    let treasury = treasury_split(&log).await;
    Backends {
        log,
        core,
        quote,
        treasury,
        unused: url(free_addr()),
    }
}

struct Proc {
    child: Child,
    addr: SocketAddr,
    _dir: PathBuf,
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts the binary with `extra_toml` appended to `[appcfg]` and `env` set; returns
/// it once it listens, or its exit status and stderr if it exits first.
async fn start(b: &Backends, extra_toml: &str, env: &[(&str, &str)]) -> Result<Proc, String> {
    let addr = free_addr();
    let dir = std::env::temp_dir().join(format!("agg-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let toml = format!(
        "bind_address = \"{addr}\"\nlog_level = \"INFO\"\n\n[appcfg]\n\
         core_url = \"{}\"\ncore_admin_url = \"{}\"\nquotes_admin_url = \"{}\"\n\
         ebill_url = \"{}\"\nclowder_url = \"{}\"\ntreasury_admin_url = \"{}\"\n{extra_toml}\n",
        b.core.web, b.core.admin, b.quote.admin, b.unused, b.unused, b.treasury.admin
    );
    std::fs::write(dir.join("config.toml"), toml).unwrap();
    let mut cmd = Command::new(bin());
    cmd.current_dir(&dir)
        .env_clear()
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn binary");
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(20) {
        if let Some(st) = child.try_wait().unwrap() {
            let mut err = String::new();
            use std::io::Read;
            child.stderr.take().unwrap().read_to_string(&mut err).unwrap();
            return Err(format!("exited {st}: {}", err.trim()));
        }
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return Ok(Proc {
                child,
                addr,
                _dir: dir,
            });
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = child.kill();
    Err("did not listen within 20s".into())
}

async fn get(addr: SocketAddr, path: &str, auth: Option<&str>) -> u16 {
    let mut rb = reqwest::Client::new().get(format!("http://{addr}{path}"));
    if let Some(a) = auth {
        rb = rb.header("authorization", a);
    }
    rb.send().await.expect("request").status().as_u16()
}

/// (no header, wrong bearer, right bearer, /health without header) on FOREIGN_BALANCE.
async fn statuses(b: &Backends, p: &Proc, secret: &str) -> (u16, u16, u16, u16, bool) {
    b.log.clear();
    let none = get(p.addr, ep::FOREIGN_BALANCE, None).await;
    let wrong = get(p.addr, ep::FOREIGN_BALANCE, Some("Bearer not-the-secret")).await;
    let reached_without = b.log.hits().iter().any(|h| h.path.contains("foreign"));
    let right = get(p.addr, ep::FOREIGN_BALANCE, Some(&format!("Bearer {secret}"))).await;
    let health = get(p.addr, ep::HEALTH, None).await;
    (none, wrong, right, health, reached_without)
}

async fn assert_guarded(b: &Backends, p: &Proc, secret: &str, what: &str) {
    let s = statuses(b, p, secret).await;
    println!("{what}: none={} wrong={} right={} health={} backend reached without key={}\n{}", s.0, s.1, s.2, s.3, s.4, b.log.dump());
    assert_eq!((s.0, s.1, s.3, s.4), (401, 401, 200, false), "{what}: unauthenticated access");
    assert_ne!(s.2, 401, "{what}: the configured secret was refused");
    assert!(
        b.log.hits().iter().any(|h| h.path.contains("foreign") && !h.unserved),
        "{what}: right key never reached treasury"
    );
}

#[tokio::test]
async fn secret_from_documented_env_var() {
    let b = backends().await;
    let p = start(&b, "", &[("ADMIN_AGGREGATOR_APPCFG__ADMIN_API_KEY", "env-secret-1")])
        .await
        .expect("start");
    assert_guarded(&b, &p, "env-secret-1", "env ADMIN_AGGREGATOR_APPCFG__ADMIN_API_KEY").await;
}

#[tokio::test]
async fn secret_from_config_toml() {
    let b = backends().await;
    let p = start(&b, "admin_api_key = \"toml-secret-1\"", &[]).await.expect("start");
    assert_guarded(&b, &p, "toml-secret-1", "config.toml appcfg.admin_api_key").await;
}

#[tokio::test]
async fn env_overrides_config_toml() {
    let b = backends().await;
    let p = start(
        &b,
        "admin_api_key = \"toml-secret-2\"",
        &[("ADMIN_AGGREGATOR_APPCFG__ADMIN_API_KEY", "env-secret-2")],
    )
    .await
    .expect("start");
    assert_guarded(&b, &p, "env-secret-2", "env over toml").await;
    let old = get(p.addr, ep::FOREIGN_BALANCE, Some("Bearer toml-secret-2")).await;
    assert_eq!(old, 401, "overridden toml secret still accepted");
}

#[tokio::test]
async fn digits_only_secret() {
    let b = backends().await;
    let p = start(&b, "", &[("ADMIN_AGGREGATOR_APPCFG__ADMIN_API_KEY", "0012345")])
        .await
        .expect("start");
    assert_guarded(&b, &p, "0012345", "digits-only env secret").await;
}

/// No secret configured, or an empty one: the aggregator must not serve.
#[tokio::test]
async fn missing_or_empty_secret_fails_closed() {
    let b = backends().await;
    let mut served = vec![];
    for (what, toml, env) in [
        ("unset", "", vec![]),
        ("empty env", "", vec![("ADMIN_AGGREGATOR_APPCFG__ADMIN_API_KEY", "")]),
        ("empty toml", "admin_api_key = \"\"", vec![]),
        ("whitespace env", "", vec![("ADMIN_AGGREGATOR_APPCFG__ADMIN_API_KEY", "   ")]),
        (
            "empty env over toml",
            "admin_api_key = \"toml-secret-3\"",
            vec![("ADMIN_AGGREGATOR_APPCFG__ADMIN_API_KEY", "")],
        ),
    ] {
        match start(&b, toml, &env).await {
            Err(e) => println!("{what}: refused to start: {e}"),
            Ok(p) => {
                let s = statuses(&b, &p, "").await;
                let bare = get(p.addr, ep::FOREIGN_BALANCE, Some("Bearer ")).await;
                let old = get(p.addr, ep::FOREIGN_BALANCE, Some("Bearer toml-secret-3")).await;
                println!("{what}: STARTED none={} wrong={} bare-bearer={bare} toml-secret={old}", s.0, s.1);
                if s.0 != 401 || s.1 != 401 || bare != 401 {
                    served.push(what);
                }
            }
        }
    }
    assert!(served.is_empty(), "aggregator served without a usable secret: {served:?}");
}

/// Every method on protected paths, without credentials, must be refused before any
/// backend is reached; only `/health` and the swagger UI answer without a key.
#[tokio::test]
async fn every_method_without_key_is_refused() {
    let b = backends().await;
    let p = start(&b, "admin_api_key = \"m-secret\"", &[]).await.expect("start");
    let qid = uuid::Uuid::new_v4().to_string();
    let paths = [
        ep::FOREIGN_BALANCE.to_string(),
        ep::ENABLE_QUOTE_MINTING.replace("{qid}", &qid),
        ep::MINT_INFO.to_string(),
    ];
    let http = reqwest::Client::new();
    let mut leaks = vec![];
    b.log.clear();
    for path in &paths {
        for m in ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "TRACE"] {
            let s = http
                .request(reqwest::Method::from_bytes(m.as_bytes()).unwrap(), format!("http://{}{path}", p.addr))
                .send()
                .await
                .expect("request")
                .status()
                .as_u16();
            println!("{m} {path} -> {s}");
            if (200..300).contains(&s) {
                leaks.push(format!("{m} {path} -> {s}"));
            }
        }
    }
    let reached: Vec<_> = b.log.hits().into_iter().map(|h| format!("{} {}", h.method, h.path)).collect();
    assert!(leaks.is_empty() && reached.is_empty(), "unauthenticated: {leaks:?}; backend reached: {reached:?}");
}

/// Path spellings and credential shapes that are not `Authorization: Bearer <key>`
/// must neither get a 2xx nor reach a backend.
#[tokio::test]
async fn path_and_credential_variants_without_key_are_refused() {
    let b = backends().await;
    let secret = "Vari4nt-Secret";
    let p = start(&b, &format!("admin_api_key = \"{secret}\""), &[]).await.expect("start");
    let fb = ep::FOREIGN_BALANCE;
    let paths = [
        format!("{fb}/"),
        format!("/{fb}"),
        fb.replace("/admin/", "//admin/"),
        fb.replace("/admin/", "/./admin/"),
        fb.replace("/admin/", "/x/../admin/"),
        fb.replace("/admin/", "%2Fadmin/"),
        fb.replace("foreign", "%66oreign"),
        fb.to_uppercase(),
        format!("{fb}?admin_api_key={secret}"),
        format!("{fb};x"),
    ];
    let creds: Vec<Vec<(&str, String)>> = vec![
        vec![("authorization", format!("Basic {secret}"))],
        vec![("authorization", format!("Token {secret}"))],
        vec![("authorization", secret.to_string())],
        vec![("authorization", format!("Bearer {}", secret.to_lowercase()))],
        vec![("authorization", format!("Bearer {}", &secret[..secret.len() - 1]))],
        vec![("authorization", format!("Bearer {secret}x"))],
        vec![("authorization", format!("Bearer {secret} {secret}"))],
        vec![("authorization", format!("Bearer{secret}"))],
        vec![("authorization", "Bearer".to_string())],
        vec![("authorization", "Bearer x".to_string()), ("authorization", format!("Bearer {secret}"))],
        vec![("proxy-authorization", format!("Bearer {secret}"))],
        vec![("x-api-key", secret.to_string())],
        vec![("cookie", format!("admin_api_key={secret}"))],
    ];
    let http = reqwest::Client::new();
    let mut leaks = vec![];
    b.log.clear();
    for path in &paths {
        let s = http.get(format!("http://{}{path}", p.addr)).send().await.expect("req").status().as_u16();
        println!("GET {path} (no key) -> {s}");
        if (200..300).contains(&s) {
            leaks.push(format!("{path} -> {s}"));
        }
    }
    for c in &creds {
        let mut rb = http.get(format!("http://{}{fb}", p.addr));
        for (k, v) in c {
            rb = rb.header(*k, v);
        }
        let s = rb.send().await.expect("req").status().as_u16();
        println!("GET {fb} {c:?} -> {s}");
        if (200..300).contains(&s) {
            leaks.push(format!("{c:?} -> {s}"));
        }
    }
    let reached: Vec<_> = b.log.hits().into_iter().map(|h| format!("{} {}", h.method, h.path)).collect();
    let right = get(p.addr, fb, Some(&format!("Bearer {secret}"))).await;
    println!("control: right key -> {right}");
    assert_ne!(right, 401, "control: right key refused");
    assert!(leaks.is_empty() && reached.is_empty(), "unauthenticated: {leaks:?}; backend reached: {reached:?}");
}

/// A secret with an inner space or punctuation is accepted at startup, so its correct
/// caller must be able to present it.
#[tokio::test]
async fn inner_space_and_punctuation_secrets_are_presentable() {
    let b = backends().await;
    for secret in ["two words", "a=b+c/d==", "with\\\"quote", "tab\\there"] {
        let p = match start(&b, &format!("admin_api_key = \"{secret}\""), &[]).await {
            Ok(p) => p,
            Err(e) => {
                println!("{secret:?}: refused at startup: {e}");
                continue;
            }
        };
        let raw = secret.replace("\\\"", "\"").replace("\\t", "\t");
        let s = get(p.addr, ep::FOREIGN_BALANCE, Some(&format!("Bearer {raw}"))).await;
        println!("{raw:?}: right key -> {s}");
        assert_ne!(s, 401, "secret {raw:?} accepted at startup but its correct caller is refused");
    }
}
