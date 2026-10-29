//! D42：Counter/Archive 的 `thing_ref` 可见性端到端。
//!
//! 默认 `read-gate`（只给有无条件 reads/consumes 的调用者），`domain` 公开，
//! `hidden` 永不返回；token 存在性不受影响。

use pact_engine::{Engine, EngineConfig, LogicalClock, SequentialIdGen};
use pact_http::client::{Client, ClientError};
use pact_http::server::{router, serve_unix, ConnTracker, ServerState};
use pact_http::visibility::VisibilityProfiles;
use pact_store::{ReadAuditKind, Store, ViewName};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;

const DECL: &str = r#"
types { requirement; design; implementation }
preconditions { requirement → design; design → implementation }
gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: consumes design, produces implementation;
    user: consumes implementation
}
seed_authority { external: seeds requirement }
"#;

const READ_GATE_BINDINGS: &str = r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: proj
spec:
  defaults:
    ref:
      visibility: read-gate
  types:
    requirement:
      ref:
        visibility: read-gate
    design:
      ref:
        visibility: read-gate
    implementation:
      ref:
        visibility: hidden
"#;

const DOMAIN_BINDINGS: &str = r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: proj
spec:
  types:
    requirement:
      ref:
        visibility: domain
"#;

const STRICT_ENTRY_BINDINGS: &str = r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: proj
spec:
  types:
    requirement:
      ref:
        required: true
        allowIdentifier: false
        schemes: ["https"]
        maxLength: 32
"#;

/// 绕过客户端直接发 REST 请求，用于覆盖“字段缺失”。
async fn raw_post(sock: &Path, path: &str, bearer: &str, body: &str) -> (u16, String) {
    let mut stream = tokio::net::UnixStream::connect(sock).await.unwrap();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {bearer}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8(raw).unwrap();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .unwrap_or_default();
    (status, body)
}

fn tokens() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("external".to_owned(), "ext-token".to_owned()),
        ("architect".to_owned(), "arch-token".to_owned()),
        ("developer".to_owned(), "dev-token".to_owned()),
        ("user".to_owned(), "user-token".to_owned()),
    ])
}

async fn spawn(
    dir: &Path,
    tokens: BTreeMap<String, String>,
    bindings: Option<&str>,
) -> (PathBuf, Engine) {
    spawn_with_config(dir, tokens, bindings, EngineConfig::default()).await
}

async fn spawn_with_config(
    dir: &Path,
    tokens: BTreeMap<String, String>,
    bindings: Option<&str>,
    config: EngineConfig,
) -> (PathBuf, Engine) {
    let store = Store::open(dir.join("data")).await.unwrap();
    let engine = Engine::spawn_with_config(
        store,
        Arc::new(LogicalClock::new()),
        Arc::new(SequentialIdGen::new()),
        config,
    );
    let profiles = match bindings {
        Some(text) => {
            let path = dir.join("refs.yaml");
            std::fs::write(&path, text).unwrap();
            VisibilityProfiles::load(&[path]).unwrap()
        }
        None => VisibilityProfiles::empty(),
    };
    let state = ServerState::with_identity_and_visibility(
        engine.clone(),
        tokens,
        BTreeMap::new(),
        BTreeMap::new(),
        None,
        None,
        profiles,
    );
    let app = router(state);
    let sock = dir.join("pact.sock");
    let server_sock = sock.clone();
    tokio::spawn(async move {
        let _ = serve_unix(
            app,
            &server_sock,
            watch::channel(false).1,
            ConnTracker::default(),
        )
        .await;
    });
    for _ in 0..100 {
        if tokio::net::UnixStream::connect(&sock).await.is_ok() {
            return (sock, engine);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("daemon 套接字未就绪");
}

async fn setup(sock: &Path) -> (Client, Client, Client) {
    let external = Client::unix(sock).with_bearer("ext-token");
    let architect = Client::unix(sock).with_bearer("arch-token");
    let developer = Client::unix(sock).with_bearer("dev-token");
    external
        .load_domain("proj", DECL, None, false)
        .await
        .unwrap();
    let req = external
        .seed(
            "proj",
            None,
            "external",
            "requirement",
            Some("https://example.com/req"),
        )
        .await
        .unwrap();
    let design = architect
        .produce(
            "proj",
            None,
            "architect",
            "design",
            vec![req.token.id.0.clone()],
            Some("https://example.com/design"),
        )
        .await
        .unwrap();
    developer
        .produce(
            "proj",
            None,
            "developer",
            "implementation",
            vec![design.token.id.0.clone()],
            Some("https://example.com/impl"),
        )
        .await
        .unwrap();
    (
        architect,
        developer,
        Client::unix(sock).with_bearer("user-token"),
    )
}

#[tokio::test]
async fn read_gate_and_hidden_filter_thing_refs() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, _engine) = spawn(dir.path(), tokens(), Some(READ_GATE_BINDINGS)).await;
    let (architect, developer, user) = setup(&sock).await;

    let view = architect.counter("proj", None, None).await.unwrap();
    assert_eq!(view.tokens.len(), 3);
    let refs: BTreeMap<_, _> = view
        .tokens
        .iter()
        .map(|token| (token.ty.0.clone(), token.thing_ref.0.clone()))
        .collect();
    assert_eq!(
        refs["requirement"].as_deref(),
        Some("https://example.com/req"),
        "reads requirement → 可见"
    );
    assert!(refs["design"].is_none(), "produce 不等于 reads/consumes");
    assert!(refs["implementation"].is_none(), "hidden");

    let view = developer.counter("proj", None, None).await.unwrap();
    let refs: BTreeMap<_, _> = view
        .tokens
        .iter()
        .map(|token| (token.ty.0.clone(), token.thing_ref.0.clone()))
        .collect();
    assert!(refs["requirement"].is_none(), "无 requirement gate");
    assert_eq!(
        refs["design"].as_deref(),
        Some("https://example.com/design"),
        "consumes design → 可见"
    );
    assert!(refs["implementation"].is_none(), "hidden");

    let view = user.counter("proj", None, None).await.unwrap();
    assert_eq!(view.tokens.len(), 3, "token 存在性不变");
    assert!(view.tokens.iter().all(|token| token.thing_ref.0.is_none()));

    // 令牌模式下匿名读取视图 → 401（不能借匿名绕过 read-gate）。
    let anonymous = Client::unix(&sock);
    match anonymous.counter("proj", None, None).await {
        Err(ClientError::Api(detail)) => assert_eq!(detail.code, "Unauthorized"),
        other => panic!("预期 401，实际 {other:?}"),
    }

    // Archive 走同一策略：developer 消费 design 后可见票面，architect 不可见。
    let archived = developer
        .counter("proj", None, Some("design"))
        .await
        .unwrap();
    let design_id = archived.tokens[0].id.0.clone();
    developer
        .consume("proj", None, "developer", &design_id, None)
        .await
        .unwrap();
    let archive = developer.archive("proj", None, None).await.unwrap();
    assert_eq!(archive.entries.len(), 1);
    assert_eq!(
        archive.entries[0].token.thing_ref.0.as_deref(),
        Some("https://example.com/design")
    );
    let archive = architect.archive("proj", None, None).await.unwrap();
    assert!(archive.entries[0].token.thing_ref.0.is_none());
}

#[tokio::test]
async fn domain_profile_is_visible_without_gate() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, _engine) = spawn(dir.path(), tokens(), Some(DOMAIN_BINDINGS)).await;
    let (_architect, _developer, user) = setup(&sock).await;

    let view = user.counter("proj", None, None).await.unwrap();
    let refs: BTreeMap<_, _> = view
        .tokens
        .iter()
        .map(|token| (token.ty.0.clone(), token.thing_ref.0.clone()))
        .collect();
    assert_eq!(
        refs["requirement"].as_deref(),
        Some("https://example.com/req"),
        "domain 覆盖无需 gate"
    );
    assert!(refs["design"].is_none(), "未声明的类型回退 read-gate");
}

#[tokio::test]
async fn trust_mode_defaults_to_read_gate() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, _engine) = spawn(dir.path(), BTreeMap::new(), None).await;
    let c = Client::unix(&sock);
    c.load_domain("proj", DECL, None, false).await.unwrap();
    c.seed(
        "proj",
        None,
        "external",
        "requirement",
        Some("https://example.com/req"),
    )
    .await
    .unwrap();

    let view = c.counter("proj", None, None).await.unwrap();
    assert_eq!(view.tokens.len(), 1);
    assert!(
        view.tokens[0].thing_ref.0.is_none(),
        "信任模式无主体 → read-gate 隐藏票面"
    );

    // 显式 domain 覆盖在信任模式下仍然可见。
    let dir = tempfile::tempdir().unwrap();
    let (sock, _engine) = spawn(dir.path(), BTreeMap::new(), Some(DOMAIN_BINDINGS)).await;
    let c = Client::unix(&sock);
    c.load_domain("proj", DECL, None, false).await.unwrap();
    c.seed(
        "proj",
        None,
        "external",
        "requirement",
        Some("https://example.com/req"),
    )
    .await
    .unwrap();
    let view = c.counter("proj", None, None).await.unwrap();
    assert_eq!(
        view.tokens[0].thing_ref.0.as_deref(),
        Some("https://example.com/req")
    );
}

/// D43：视图读取按类型记录暴露计数；审计流同时保留视图与 Rule-R 记录。
#[tokio::test]
async fn view_read_audit_records_exposure_per_type() {
    let dir = tempfile::tempdir().unwrap();
    let config = EngineConfig {
        read_audit: true,
        ..EngineConfig::default()
    };
    let (sock, _engine) =
        spawn_with_config(dir.path(), tokens(), Some(READ_GATE_BINDINGS), config).await;
    let (architect, developer, _user) = setup(&sock).await;

    let view = architect.counter("proj", None, None).await.unwrap();
    assert_eq!(view.tokens.len(), 3);

    let records = architect
        .read_audit("proj", None, None)
        .await
        .unwrap()
        .records;
    assert_eq!(records.len(), 1);
    let rec = &records[0];
    assert_eq!(rec.kind, ReadAuditKind::View);
    assert_eq!(rec.view, Some(ViewName::Counter));
    assert_eq!(rec.participant.as_deref(), Some("architect"));
    assert!(rec.token_id.is_none());
    let by_type: BTreeMap<_, _> = rec
        .types
        .as_ref()
        .unwrap()
        .iter()
        .map(|ty| (ty.ty.as_str(), ty))
        .collect();
    assert_eq!(by_type["requirement"].refs_visible, 1);
    assert_eq!(by_type["requirement"].refs_filtered, 0);
    assert_eq!(by_type["design"].refs_visible, 0);
    assert_eq!(by_type["design"].refs_filtered, 1);
    assert_eq!(by_type["implementation"].refs_filtered, 1);

    // Archive 读取同样审计；design 已由 developer 消费并可见。
    let design_id = view
        .tokens
        .iter()
        .find(|token| token.ty.0 == "design")
        .unwrap()
        .id
        .0
        .clone();
    developer
        .consume("proj", None, "developer", &design_id, None)
        .await
        .unwrap();
    developer.archive("proj", None, None).await.unwrap();

    let records = developer
        .read_audit("proj", None, None)
        .await
        .unwrap()
        .records;
    let rec = records.last().unwrap();
    assert_eq!(rec.view, Some(ViewName::Archive));
    assert_eq!(rec.participant.as_deref(), Some("developer"));
    let by_type: BTreeMap<_, _> = rec
        .types
        .as_ref()
        .unwrap()
        .iter()
        .map(|ty| (ty.ty.as_str(), ty))
        .collect();
    assert_eq!(by_type["design"].refs_visible, 1);
    assert_eq!(by_type["design"].refs_filtered, 0);
}

/// D43：信任模式匿名视图审计 `participant = null`；审计写失败时视图以 500 拒绝。
#[tokio::test]
async fn view_audit_is_anonymous_in_trust_mode_and_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let config = EngineConfig {
        read_audit: true,
        ..EngineConfig::default()
    };
    let (sock, _engine) =
        spawn_with_config(dir.path(), BTreeMap::new(), Some(DOMAIN_BINDINGS), config).await;
    let c = Client::unix(&sock);
    c.load_domain("proj", DECL, None, false).await.unwrap();
    c.seed(
        "proj",
        None,
        "external",
        "requirement",
        Some("https://example.com/req"),
    )
    .await
    .unwrap();
    c.counter("proj", None, None).await.unwrap();

    let records = c.read_audit("proj", None, None).await.unwrap().records;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind, ReadAuditKind::View);
    assert!(
        records[0].participant.is_none(),
        "信任模式无主体，审计如实记录匿名"
    );

    // 审计写入失败 → 视图以 500 拒绝（不返回未经审计的视图）。
    let audit = dir
        .path()
        .join("data")
        .join("proj")
        .join("default.read-audit.jsonl");
    std::fs::remove_file(&audit).unwrap();
    std::fs::create_dir_all(&audit).unwrap();
    match c.counter("proj", None, None).await {
        Err(ClientError::Api(detail)) => assert_eq!(detail.code, "Store"),
        other => panic!("预期审计失败返回 500，实际 {other:?}"),
    }
}

#[tokio::test]
async fn entry_validation_structural_rules() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, _engine) = spawn(dir.path(), tokens(), None).await;
    let c = Client::unix(&sock).with_bearer("ext-token");
    c.load_domain("proj", DECL, None, false).await.unwrap();

    // 字段缺失 → 400；显式 null → ∅ 并成功。
    let (status, body) = raw_post(
        &sock,
        "/domains/proj/seeds",
        "ext-token",
        r#"{"participant":"external","type":"requirement"}"#,
    )
    .await;
    assert_eq!(status, 400, "body: {body}");
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["error"]["code"], "ThingRefMissing");

    let (status, body) = raw_post(
        &sock,
        "/domains/proj/seeds",
        "ext-token",
        r#"{"participant":"external","type":"requirement","thing_ref":null}"#,
    )
    .await;
    assert_eq!(status, 201, "body: {body}");

    let long = "x".repeat(2049);
    for raw in [
        "https://user:pass@example.com/a",
        "/tmp/local.md",
        "foo bar",
        long.as_str(),
    ] {
        match c
            .seed("proj", None, "external", "requirement", Some(raw))
            .await
        {
            Err(ClientError::Api(detail)) => {
                assert_eq!(detail.code, "InvalidThingRef", "raw={raw:?}")
            }
            other => panic!("预期拒绝 {raw:?}，实际 {other:?}"),
        }
    }

    let token = c
        .seed(
            "proj",
            None,
            "external",
            "requirement",
            Some("HTTPS://Example.com/a"),
        )
        .await
        .unwrap();
    assert_eq!(
        token.token.thing_ref.0.as_deref(),
        Some("https://Example.com/a")
    );
}

#[tokio::test]
async fn entry_validation_applies_bindings_policy() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, _engine) = spawn(dir.path(), tokens(), Some(STRICT_ENTRY_BINDINGS)).await;
    let c = Client::unix(&sock).with_bearer("ext-token");
    c.load_domain("proj", DECL, None, false).await.unwrap();

    match c.seed("proj", None, "external", "requirement", None).await {
        Err(ClientError::Api(detail)) => assert_eq!(detail.code, "ThingRefRequired"),
        other => panic!("required 域应拒绝 ∅，实际 {other:?}"),
    }
    for raw in [
        "REQ-1",
        "git://example.com/a",
        "https://example.com/aaaaaaaaaaaaaaaaaaaa",
    ] {
        match c
            .seed("proj", None, "external", "requirement", Some(raw))
            .await
        {
            Err(ClientError::Api(detail)) => {
                assert_eq!(detail.code, "InvalidThingRef", "raw={raw}")
            }
            other => panic!("预期拒绝 {raw:?}，实际 {other:?}"),
        }
    }
    c.seed(
        "proj",
        None,
        "external",
        "requirement",
        Some("https://example.com/a"),
    )
    .await
    .unwrap();
}
