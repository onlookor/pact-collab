//! NFR6 v2 集成测试（spec §5.12 / §7）：TLS、Ed25519 签名、MCP Streamable HTTP、OIDC。
//!
//! 进程内起真实 daemon（unix socket / TCP / TLS），客户端分别使用类型化 client、
//! 原始 hyper HTTP/1.1 与 Ed25519 JWT，验证四块安全能力的边界行为。

use axum::routing::get;
use axum::Router;
use base64::Engine as _;
use bytes::Bytes;
use http::{HeaderMap, Method, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use pact_engine::{Engine, LogicalClock, SequentialIdGen};
use pact_http::client::{Client, ClientError};
use pact_http::config::{OidcSettings, TlsConfig};
use pact_http::oidc::{OidcError, OidcValidator};
use pact_http::server::{router, router_with_mcp, serve_tcp, serve_unix, ConnTracker, ServerState};
use pact_http::sign::SigningIdentity;
use pact_http::tls::server_acceptor;
use pact_store::Store;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

const DECL: &str = r#"
types { requirement; design; implementation }

preconditions {
    requirement → design;
    design → implementation
}

gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: consumes design, produces implementation;
    user: consumes implementation
}

seed_authority { external: seeds requirement }
"#;

/// 固定 Ed25519 种子（base64(32B)）：architect / mallory 各一。
const SEED_ARCHITECT: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const SEED_MALLORY: &str = "ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=";

/// OIDC 测试用 Ed25519 种子。
const ED_SEED: [u8; 32] = [7u8; 32];

const OK: StatusCode = StatusCode::OK;

// ============ 断言用常量 ============

const EXPECTED_TOOLS: [&str; 17] = [
    "pact_domain_validate",
    "pact_domain_load",
    "pact_domain_instantiate",
    "pact_list_domains",
    "pact_analyze",
    "pact_compose",
    "pact_seed",
    "pact_produce",
    "pact_read",
    "pact_consume",
    "pact_counter_query",
    "pact_archive_query",
    "pact_my_gates",
    "pact_enabled",
    "pact_status",
    "pact_compact",
    "pact_subscribe",
];

// ============ 进程内 daemon ============

async fn make_state(
    dir: &Path,
    tokens: BTreeMap<String, String>,
    keys: BTreeMap<String, [u8; 32]>,
    oidc: Option<Arc<OidcValidator>>,
    public_url: Option<String>,
) -> Arc<ServerState> {
    let store = Store::open(dir.join("data")).await.unwrap();
    let engine = Engine::spawn(
        store,
        Arc::new(LogicalClock::new()),
        Arc::new(SequentialIdGen::new()),
    );
    ServerState::with_identity(engine, tokens, keys, BTreeMap::new(), oidc, public_url)
}

async fn spawn_unix(dir: &Path, name: &str, app: Router) -> PathBuf {
    let sock = dir.join(name);
    let s = sock.clone();
    tokio::spawn(async move {
        let _ = serve_unix(app, &s, watch::channel(false).1, ConnTracker::default()).await;
    });
    for _ in 0..100 {
        if tokio::net::UnixStream::connect(&sock).await.is_ok() {
            return sock;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("unix daemon 未就绪");
}

/// `serve_tcp` 内部 bind，无法取 0 端口：先探测端口后释放，再用该端口监听。
async fn spawn_tcp(app: Router, tls: Option<tokio_rustls::TlsAcceptor>) -> String {
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = probe.local_addr().unwrap().to_string();
    drop(probe);
    let a = addr.clone();
    tokio::spawn(async move {
        let _ = serve_tcp(
            app,
            &a,
            tls,
            watch::channel(false).1,
            ConnTracker::default(),
        )
        .await;
    });
    for _ in 0..200 {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            return addr;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("TCP daemon 未就绪");
}

// ============ 原始 HTTP 客户端 ============

fn is_sse(headers: &HeaderMap) -> bool {
    headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/event-stream"))
}

fn has_jsonrpc_result(buf: &[u8]) -> bool {
    String::from_utf8_lossy(buf)
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .any(|payload| {
            serde_json::from_str::<serde_json::Value>(payload.trim())
                .is_ok_and(|v| v.get("result").is_some() || v.get("error").is_some())
        })
}

/// 读响应体：SSE 在拿到 JSON-RPC 结果后即停（请求级流可能不主动关闭）。
async fn read_body(mut body: Incoming, sse: bool) -> Bytes {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, body.frame()).await {
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    out.extend_from_slice(data);
                }
                if sse && has_jsonrpc_result(&out) {
                    break;
                }
            }
            _ => break,
        }
    }
    Bytes::from(out)
}

async fn send_unix_request(
    sock: &Path,
    req: Request<Full<Bytes>>,
) -> (StatusCode, HeaderMap, Bytes) {
    let stream = tokio::net::UnixStream::connect(sock).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let resp = sender.send_request(req).await.unwrap();
    collect(resp).await
}

async fn send_tcp_request(addr: &str, req: Request<Full<Bytes>>) -> (StatusCode, HeaderMap, Bytes) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let resp = sender.send_request(req).await.unwrap();
    collect(resp).await
}

async fn collect(resp: hyper::Response<Incoming>) -> (StatusCode, HeaderMap, Bytes) {
    let (parts, body) = resp.into_parts();
    let sse = is_sse(&parts.headers);
    let bytes = read_body(body, sse).await;
    (parts.status, parts.headers, bytes)
}

fn get_request(uri: &str) -> Request<Full<Bytes>> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Full::new(Bytes::new()))
        .unwrap()
}

// ============ 签名请求 ============

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn public_key(id: &SigningIdentity) -> [u8; 32] {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(id.public_key_base64())
        .unwrap();
    bytes.try_into().unwrap()
}

/// 用 `signed_body` 生成签名头，但实际发送 `sent_body`（篡改测试用）。
fn signed_request(
    id: &SigningIdentity,
    path_and_query: &str,
    signed_body: &[u8],
    sent_body: &[u8],
    nonce: &str,
) -> Request<Full<Bytes>> {
    let h = id.headers("POST", path_and_query, signed_body, now_secs(), nonce);
    Request::builder()
        .method(Method::POST)
        .uri(format!("http://pactd{path_and_query}"))
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(pact_http::sign::HEADER_PARTICIPANT, h.participant)
        .header(pact_http::sign::HEADER_TIMESTAMP, h.timestamp.to_string())
        .header(pact_http::sign::HEADER_NONCE, h.nonce)
        .header(pact_http::sign::HEADER_SIGNATURE, h.signature)
        .body(Full::new(Bytes::copy_from_slice(sent_body)))
        .unwrap()
}

// ============ TLS 证书 ============

struct TlsFiles {
    cert: PathBuf,
    key: PathBuf,
    ca: PathBuf,
}

fn make_ca() -> (rcgen::Certificate, KeyPair) {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params
        .distinguished_name
        .push(DnType::CommonName, "PACT Test CA");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let key = KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    (cert, key)
}

fn write_tls_files(dir: &Path) -> TlsFiles {
    let (ca_cert, ca_key) = make_ca();
    let mut params =
        CertificateParams::new(vec!["127.0.0.1".to_owned(), "localhost".to_owned()]).unwrap();
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_key = KeyPair::generate().unwrap();
    let server_cert = params.signed_by(&server_key, &ca_cert, &ca_key).unwrap();

    let ca = dir.join("ca.pem");
    let cert = dir.join("server.pem");
    let key = dir.join("server.key");
    std::fs::write(&ca, ca_cert.pem()).unwrap();
    std::fs::write(&cert, server_cert.pem()).unwrap();
    std::fs::write(&key, server_key.serialize_pem()).unwrap();
    TlsFiles { cert, key, ca }
}

// ============ MCP over HTTP ============

fn initialize_body() -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "nfr6-v2-test", "version": "0.1.0" }
        }
    })
}

async fn mcp_post(
    addr: &str,
    token: Option<&str>,
    body: &serde_json::Value,
    session: Option<&str>,
) -> (StatusCode, HeaderMap, Bytes) {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(format!("http://{addr}/mcp"))
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(http::header::ACCEPT, "application/json, text/event-stream");
    if let Some(t) = token {
        builder = builder.header(http::header::AUTHORIZATION, format!("Bearer {t}"));
    }
    if let Some(s) = session {
        builder = builder.header("mcp-session-id", s);
    }
    let req = builder
        .body(Full::new(Bytes::from(serde_json::to_vec(body).unwrap())))
        .unwrap();
    send_tcp_request(addr, req).await
}

/// 解析 JSON-RPC 响应（SSE 取含 result/error 的 data 行，JSON 直接解析）。
fn jsonrpc_message(headers: &HeaderMap, body: &Bytes) -> serde_json::Value {
    if is_sse(headers) {
        for line in String::from_utf8_lossy(body).lines() {
            let Some(rest) = line.strip_prefix("data:") else {
                continue;
            };
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(rest.trim()) {
                if v.get("result").is_some() || v.get("error").is_some() {
                    return v;
                }
            }
        }
        panic!("SSE 响应未包含 JSON-RPC 结果：{body:?}");
    }
    serde_json::from_slice(body).unwrap_or_else(|e| panic!("响应体不是合法 JSON：{e}"))
}

// ============ OIDC mock IdP ============

fn ed25519_jwk() -> serde_json::Value {
    let key = ed25519_dalek::SigningKey::from_bytes(&ED_SEED);
    let x = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes());
    serde_json::json!({
        "kty": "OKP",
        "crv": "Ed25519",
        "kid": "k1",
        "alg": "EdDSA",
        "x": x,
    })
}

/// 启动 mock IdP，返回 issuer 基址（无尾随斜杠）。
async fn start_idp() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let discovery_base = base.clone();
    let jwks = serde_json::json!({ "keys": [ed25519_jwk()] });
    let app = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(move || {
                let b = discovery_base.clone();
                async move {
                    axum::Json(serde_json::json!({
                        "issuer": b.clone(),
                        "jwks_uri": format!("{b}/jwks"),
                    }))
                }
            }),
        )
        .route(
            "/jwks",
            get(move || {
                let keys = jwks.clone();
                async move { axum::Json(keys) }
            }),
        );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    base
}

const ED_PKCS8_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

fn ed_pkcs8(seed: &[u8; 32]) -> Vec<u8> {
    let mut der = ED_PKCS8_PREFIX.to_vec();
    der.extend_from_slice(seed);
    der
}

fn sign_jwt(kid: &str, claims: &serde_json::Value) -> String {
    let header = jsonwebtoken::Header {
        alg: jsonwebtoken::Algorithm::EdDSA,
        kid: Some(kid.to_owned()),
        ..Default::default()
    };
    let key = jsonwebtoken::EncodingKey::from_ed_der(&ed_pkcs8(&ED_SEED));
    jsonwebtoken::encode(&header, claims, &key).unwrap()
}

fn claims(issuer: &str, audience: &str, exp_offset: i64) -> serde_json::Value {
    let exp = i64::try_from(now_secs()).unwrap() + exp_offset;
    serde_json::json!({
        "iss": issuer,
        "aud": audience,
        "sub": "user-1",
        "exp": exp,
    })
}

fn oidc_settings(issuer: &str, jwks_uri: Option<String>, discovery: bool) -> OidcSettings {
    OidcSettings {
        issuer: issuer.to_owned(),
        audience: "pact".to_owned(),
        principal_claim: "sub".to_owned(),
        principal_map: BTreeMap::new(),
        jwks_uri,
        discovery,
        allowed_algs: vec!["EdDSA".to_owned()],
        ca: None,
        allow_static_tokens: false,
        allow_insecure_http: true,
    }
}

// ============ 1. TLS ============

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tls_roundtrip_with_custom_ca() {
    let dir = tempfile::tempdir().unwrap();
    let tls = write_tls_files(dir.path());
    let state = make_state(dir.path(), BTreeMap::new(), BTreeMap::new(), None, None).await;
    let acceptor = server_acceptor(&TlsConfig {
        certificate: Some(tls.cert),
        private_key: Some(tls.key),
        client_ca: None,
    })
    .unwrap();
    let addr = spawn_tcp(router(state), Some(acceptor)).await;

    let c = Client::https(addr, Some(tls.ca)).unwrap();
    let v = c.validate(DECL).await.unwrap();
    assert!(v.ok, "HTTPS 校验声明应成功");
    assert_eq!(v.types.len(), 3);
    assert_eq!(v.participants.len(), 4);
}

// ============ 2. mTLS ============

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mtls_rejects_client_without_cert() {
    let dir = tempfile::tempdir().unwrap();
    let tls = write_tls_files(dir.path());
    let state = make_state(dir.path(), BTreeMap::new(), BTreeMap::new(), None, None).await;
    let acceptor = server_acceptor(&TlsConfig {
        certificate: Some(tls.cert),
        private_key: Some(tls.key),
        client_ca: Some(tls.ca.clone()),
    })
    .unwrap();
    let addr = spawn_tcp(router(state), Some(acceptor)).await;

    let c = Client::https(addr, Some(tls.ca)).unwrap();
    let res = tokio::time::timeout(Duration::from_secs(2), c.validate(DECL)).await;
    assert!(
        matches!(res, Ok(Err(_)) | Err(_)),
        "无客户端证书的连接必须在 2s 内失败"
    );
}

// ============ 3. 签名必需且可用 ============

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signature_required_and_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let architect = SigningIdentity::from_seed_base64("architect", SEED_ARCHITECT).unwrap();
    let mut keys = BTreeMap::new();
    keys.insert("architect".to_owned(), public_key(&architect));
    let state = make_state(dir.path(), BTreeMap::new(), keys, None, None).await;
    let sock = spawn_unix(dir.path(), "pact.sock", router(state)).await;

    let unsigned = Client::unix(sock.clone());
    let err = unsigned
        .load_domain("proj", DECL, None, false)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ClientError::Api(d) if d.code == "Unauthorized"),
        "无签名请求应 401，实际 {err}"
    );

    let signed = Client::unix(sock).with_signing(architect);
    let summary = signed.load_domain("proj", DECL, None, false).await.unwrap();
    assert_eq!(summary.name, "proj");
    assert_eq!(summary.instance, "default");
}

// ============ 4. 重放 ============

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signature_replay_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let architect = SigningIdentity::from_seed_base64("architect", SEED_ARCHITECT).unwrap();
    let mut keys = BTreeMap::new();
    keys.insert("architect".to_owned(), public_key(&architect));
    let state = make_state(dir.path(), BTreeMap::new(), keys, None, None).await;
    let sock = spawn_unix(dir.path(), "pact.sock", router(state)).await;

    let body = serde_json::to_vec(&serde_json::json!({ "declaration": DECL })).unwrap();
    let nonce = "nfr6-replay-nonce-000000001";

    let first = signed_request(&architect, "/validate", &body, &body, nonce);
    let (status, _, _) = send_unix_request(&sock, first).await;
    assert_eq!(status, OK, "首次签名请求应成功");

    // 同一 nonce 重放：nonce 去重命中 → 429 TooManyRequests（spec §4.2）。
    let second = signed_request(&architect, "/validate", &body, &body, nonce);
    let (status, _, _) = send_unix_request(&sock, second).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "重放应被拒绝");
}

// ============ 5. 篡改请求体 ============

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signature_tampered_body_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let architect = SigningIdentity::from_seed_base64("architect", SEED_ARCHITECT).unwrap();
    let mut keys = BTreeMap::new();
    keys.insert("architect".to_owned(), public_key(&architect));
    let state = make_state(dir.path(), BTreeMap::new(), keys, None, None).await;
    let sock = spawn_unix(dir.path(), "pact.sock", router(state)).await;

    let signed_body = br#"{"declaration":"types { a }"}"#;
    let sent_body = br#"{"declaration":"types { b }"}"#;
    let req = signed_request(
        &architect,
        "/validate",
        signed_body,
        sent_body,
        "nfr6-tamper-nonce-000000001",
    );
    let (status, _, _) = send_unix_request(&sock, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "篡改请求体应 401");
}

// ============ 6. 参与者不匹配 ============

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signature_participant_mismatch_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mallory = SigningIdentity::from_seed_base64("mallory", SEED_MALLORY).unwrap();
    let mut keys = BTreeMap::new();
    keys.insert("mallory".to_owned(), public_key(&mallory));
    let state = make_state(dir.path(), BTreeMap::new(), keys, None, None).await;
    let sock = spawn_unix(dir.path(), "pact.sock", router(state)).await;

    let c = Client::unix(sock).with_signing(mallory);
    let err = c
        .seed("proj", None, "architect", "requirement", None)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ClientError::Api(d) if d.code == "Unauthorized"),
        "签名身份与 participant 不一致应 401，实际 {err}"
    );
}

// ============ 7. MCP 需要 Bearer ============

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_http_requires_bearer() {
    let dir = tempfile::tempdir().unwrap();
    let mut tokens = BTreeMap::new();
    tokens.insert("tester".to_owned(), "tok".to_owned());
    let state = make_state(
        dir.path(),
        tokens,
        BTreeMap::new(),
        None,
        Some("https://pact.example.com".to_owned()),
    )
    .await;
    let addr = spawn_tcp(router_with_mcp(state, true), None).await;

    let (status, headers, _) = mcp_post(&addr, None, &initialize_body(), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let challenge = headers
        .get(http::header::WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())
        .expect("401 必须携带 WWW-Authenticate");
    assert!(
        challenge.contains("resource_metadata"),
        "challenge 应指向 RFC 9728 元数据：{challenge}"
    );
    assert!(challenge.contains("https://pact.example.com/.well-known/oauth-protected-resource"));
}

// ============ 8. MCP 初始化 + 工具清单 ============

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_http_initialize_and_tools_list() {
    let dir = tempfile::tempdir().unwrap();
    let mut tokens = BTreeMap::new();
    tokens.insert("tester".to_owned(), "tok".to_owned());
    let state = make_state(dir.path(), tokens, BTreeMap::new(), None, None).await;
    let addr = spawn_tcp(router_with_mcp(state, true), None).await;

    let (status, headers, body) = mcp_post(&addr, Some("tok"), &initialize_body(), None).await;
    assert_eq!(status, OK, "initialize 应成功");
    let session = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .expect("initialize 响应必须携带 mcp-session-id")
        .to_owned();
    let msg = jsonrpc_message(&headers, &body);
    assert!(
        msg.get("result").is_some(),
        "initialize 应返回 result：{msg}"
    );

    let initialized = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
    });
    let (status, _, _) = mcp_post(&addr, Some("tok"), &initialized, Some(&session)).await;
    assert!(status.is_success(), "notifications/initialized 应受理");

    let list = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
    });
    let (status, headers, body) = mcp_post(&addr, Some("tok"), &list, Some(&session)).await;
    assert_eq!(status, OK, "tools/list 应成功");
    let msg = jsonrpc_message(&headers, &body);
    let tools = msg["result"]["tools"]
        .as_array()
        .expect("tools/list 结果应含 tools 数组");
    assert_eq!(tools.len(), 17, "应暴露 17 个工具");
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    for expected in EXPECTED_TOOLS {
        assert!(names.contains(&expected), "缺少工具 {expected}");
    }
}

// ============ 9. 既有信任模式 ============

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_unsigned_mode_still_works() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state(dir.path(), BTreeMap::new(), BTreeMap::new(), None, None).await;
    let sock = spawn_unix(dir.path(), "pact.sock", router(state)).await;

    let (status, _, body) = send_unix_request(&sock, get_request("http://pactd/health")).await;
    assert_eq!(status, OK);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["status"],
        "ok"
    );

    let c = Client::unix(sock);
    assert!(c.validate(DECL).await.unwrap().ok);
}

// ============ 10. OIDC 接受合法令牌 ============

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_oidc_accepts_valid_token() {
    let dir = tempfile::tempdir().unwrap();
    let issuer = start_idp().await;
    let validator = OidcValidator::new(oidc_settings(&issuer, None, true)).unwrap();

    let token = sign_jwt("k1", &claims(&issuer, "pact", 300));
    assert_eq!(validator.validate(&token).await.unwrap(), "user-1");

    let state = make_state(
        dir.path(),
        BTreeMap::new(),
        BTreeMap::new(),
        Some(validator),
        Some("https://pact.example.com".to_owned()),
    )
    .await;
    let addr = spawn_tcp(router_with_mcp(state, true), None).await;

    let (status, headers, body) = mcp_post(&addr, Some(&token), &initialize_body(), None).await;
    assert_eq!(status, OK, "OIDC 令牌应可完成 initialize");
    assert!(headers.get("mcp-session-id").is_some());
    assert!(jsonrpc_message(&headers, &body).get("result").is_some());
}

// ============ 11. OIDC 过期 / 错误受众 ============

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_oidc_rejects_expired_or_wrong_audience() {
    let issuer = start_idp().await;
    let validator = OidcValidator::new(oidc_settings(&issuer, None, true)).unwrap();

    let expired = sign_jwt("k1", &claims(&issuer, "pact", -3600));
    let err = validator.validate(&expired).await.unwrap_err();
    assert!(
        matches!(err, OidcError::Invalid(_)),
        "过期令牌应判为 Invalid，实际 {err}"
    );

    let wrong_aud = sign_jwt("k1", &claims(&issuer, "other-resource", 300));
    let err = validator.validate(&wrong_aud).await.unwrap_err();
    assert!(
        matches!(err, OidcError::Invalid(_)),
        "受众不符应判为 Invalid，实际 {err}"
    );
}

// ============ 12. OIDC 拒绝 alg=none / 未知 kid ============

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_oidc_rejects_alg_none_and_unknown_kid() {
    let issuer = start_idp().await;
    let validator = OidcValidator::new(oidc_settings(&issuer, None, true)).unwrap();

    let header =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&claims(&issuer, "pact", 300)).unwrap());
    let alg_none = format!("{header}.{payload}.");
    let err = validator.validate(&alg_none).await.unwrap_err();
    assert!(
        matches!(err, OidcError::Invalid(_)),
        "alg=none 必须被拒绝，实际 {err}"
    );

    let unknown = sign_jwt("unknown", &claims(&issuer, "pact", 300));
    let err = validator.validate(&unknown).await.unwrap_err();
    assert!(
        matches!(err, OidcError::Invalid(_)),
        "未知 kid 必须被拒绝，实际 {err}"
    );
}

// ============ 13. OIDC claim 映射与非法字符 ============

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_oidc_principal_claim_mapping_and_invalid_chars() {
    let issuer = start_idp().await;

    let mut settings = oidc_settings(&issuer, None, true);
    settings.principal_claim = "email".to_owned();
    settings
        .principal_map
        .insert("alice@corp.example".to_owned(), "alice".to_owned());
    let validator = OidcValidator::new(settings).unwrap();

    let exp = i64::try_from(now_secs()).unwrap() + 300;
    let mapped = sign_jwt(
        "k1",
        &serde_json::json!({
            "iss": issuer,
            "aud": "pact",
            "sub": "user-1",
            "email": "alice@corp.example",
            "exp": exp,
        }),
    );
    assert_eq!(validator.validate(&mapped).await.unwrap(), "alice");

    // claim 原值含非法字符 → 拒绝，且错误消息不回显原值。
    let invalid_raw = sign_jwt(
        "k1",
        &serde_json::json!({
            "iss": issuer,
            "aud": "pact",
            "sub": "user-2",
            "email": "alice;drop",
            "exp": exp,
        }),
    );
    let err = validator.validate(&invalid_raw).await.unwrap_err();
    assert!(matches!(err, OidcError::Invalid(_)));
    assert!(
        !err.to_string().contains("alice;drop"),
        "错误消息不得回显 claim 原值：{err}"
    );

    // 映射目标含非法字符 → 同样拒绝且不回显。
    let mut settings = oidc_settings(&issuer, None, true);
    settings.principal_claim = "email".to_owned();
    settings
        .principal_map
        .insert("bob@corp.example".to_owned(), "alice;drop".to_owned());
    let validator = OidcValidator::new(settings).unwrap();
    let invalid_target = sign_jwt(
        "k1",
        &serde_json::json!({
            "iss": issuer,
            "aud": "pact",
            "sub": "user-3",
            "email": "bob@corp.example",
            "exp": exp,
        }),
    );
    let err = validator.validate(&invalid_target).await.unwrap_err();
    assert!(matches!(err, OidcError::Invalid(_)));
    assert!(
        !err.to_string().contains("alice;drop"),
        "错误消息不得回显映射结果：{err}"
    );
}

// ============ 14. RFC 9728 元数据 ============

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oauth_protected_resource_metadata() {
    let issuer = start_idp().await;
    let validator = OidcValidator::new(oidc_settings(&issuer, None, true)).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let state = make_state(
        dir.path(),
        BTreeMap::new(),
        BTreeMap::new(),
        Some(validator),
        Some("https://pact.example.com".to_owned()),
    )
    .await;
    let sock = spawn_unix(dir.path(), "pact.sock", router(state)).await;
    let (status, _, body) = send_unix_request(
        &sock,
        get_request("http://pactd/.well-known/oauth-protected-resource"),
    )
    .await;
    assert_eq!(status, OK);
    let meta: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(meta["resource"], "https://pact.example.com");
    let servers = meta["authorization_servers"].as_array().unwrap();
    assert!(servers.iter().any(|v| v.as_str() == Some(issuer.as_str())));

    // 未配置 OIDC → 404。
    let dir2 = tempfile::tempdir().unwrap();
    let state2 = make_state(
        dir2.path(),
        BTreeMap::new(),
        BTreeMap::new(),
        None,
        Some("https://pact.example.com".to_owned()),
    )
    .await;
    let sock2 = spawn_unix(dir2.path(), "pact.sock", router(state2)).await;
    let (status, _, _) = send_unix_request(
        &sock2,
        get_request("http://pactd/.well-known/oauth-protected-resource"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ============ 15. IdP 不可达 → 503 ============

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_oidc_idp_unreachable_returns_503() {
    let dir = tempfile::tempdir().unwrap();
    let issuer = "http://127.0.0.1:1";
    let validator =
        OidcValidator::new(oidc_settings(issuer, Some(format!("{issuer}/jwks")), false)).unwrap();
    let token = sign_jwt("k1", &claims(issuer, "pact", 300));

    let state = make_state(
        dir.path(),
        BTreeMap::new(),
        BTreeMap::new(),
        Some(validator),
        Some("https://pact.example.com".to_owned()),
    )
    .await;
    let addr = spawn_tcp(router_with_mcp(state, true), None).await;

    let (status, _, _) = mcp_post(&addr, Some(&token), &initialize_body(), None).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "IdP 不可达应返回 503"
    );
}
