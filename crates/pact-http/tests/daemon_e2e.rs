//! pact-http 端到端：真实 daemon（unix socket）+ 类型化 client 全生命周期。
//!
//! 覆盖 §6 线上格式、§9.2 端点、错误码映射、§6.4 SSE、§9.4 令牌校验。

use pact_engine::{Engine, LogicalClock, SequentialIdGen};
use pact_http::client::{Client, ClientError};
use pact_http::server::{router, serve_unix, ServerState};
use pact_store::Store;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

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

/// 进程内起一个真实 daemon（临时目录 + unix socket），返回套接字路径与引擎句柄。
async fn spawn_daemon(dir: &Path, tokens: BTreeMap<String, String>) -> (PathBuf, Engine) {
    let store = Store::open(dir.join("data")).await.unwrap();
    let engine = Engine::spawn(
        store,
        Arc::new(LogicalClock::new()),
        Arc::new(SequentialIdGen::new()),
    );
    let state = ServerState::new(engine.clone(), tokens);
    let app = router(state);
    let sock = dir.join("pact.sock");
    let s = sock.clone();
    tokio::spawn(async move {
        let _ = serve_unix(app, &s).await;
    });
    // 等套接字可连
    for _ in 0..100 {
        if tokio::net::UnixStream::connect(&sock).await.is_ok() {
            return (sock, engine);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("daemon 套接字未就绪");
}

#[tokio::test]
async fn full_lifecycle_over_unix_socket() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, _engine) = spawn_daemon(dir.path(), BTreeMap::new()).await;
    let c = Client::unix(sock);

    // —— 域生命周期 ——
    let v = c.validate(DECL).await.unwrap();
    assert!(v.ok);
    assert_eq!((v.types.len(), v.participants.len(), v.gates, v.seed_authority), (3, 4, 6, 1));
    // 非法声明 → §6.3 结构化错误（不加载）
    match c.validate("types { a }").await {
        Err(ClientError::Api(d)) => assert!(matches!(d.code.as_str(), "WellFormedness" | "Syntax")),
        other => panic!("预期良构违例，实际 {other:?}"),
    }

    let summary = c.load_domain("proj", DECL).await.unwrap();
    assert_eq!(summary.name, "proj");
    assert_eq!((summary.types, summary.gates, summary.seed_authority), (3, 6, 1));

    let list = c.list_domains().await.unwrap();
    assert_eq!(list.domains, vec!["proj".to_owned()]);

    let info = c.domain_info("proj").await.unwrap();
    assert_eq!(info.counter_size, 0);
    // Rule-T 信息性判定：Counter 空且无可执行 Produce → 成立（种子仍可注入）
    assert!(info.terminated);

    // —— Rule-S ——
    let seed = c.seed("proj", "external", "requirement", Some("REQ-1")).await.unwrap();
    assert_eq!(seed.token.ty.0, "requirement");
    let req_id = seed.token.id.0.clone();

    // —— enabled（§6.5，字段名 type）——
    let enabled = c.enabled("proj", "architect").await.unwrap();
    assert_eq!(enabled.options.len(), 1);
    assert_eq!(enabled.options[0].ty.0, "design");
    assert_eq!(enabled.options[0].witness_predecessors[0].0, req_id);

    // —— Rule-P ——
    let prod = c
        .produce("proj", "architect", "design", vec![req_id.clone()], None)
        .await
        .unwrap();
    let design_id = prod.token.id.0.clone();

    // —— Rule-R ——
    let read = c.read_token("proj", "architect", &req_id).await.unwrap();
    assert_eq!(read.token.id.0, req_id);

    // —— Rule-E ——
    let entry = c.consume("proj", "developer", &design_id).await.unwrap();
    assert_eq!(entry.entry.token.id.0, design_id);
    assert_eq!(entry.entry.exhausted_by.0, "developer");

    // —— 查询 ——
    let counter = c.counter("proj", None).await.unwrap();
    assert_eq!(counter.size, 1); // 仅剩 requirement
    let counter_f = c.counter("proj", Some("design")).await.unwrap();
    assert_eq!(counter_f.size, 0);
    let archive = c.archive("proj", None).await.unwrap();
    assert_eq!(archive.size, 1);
    let gates = c.gates("proj", "developer").await.unwrap();
    assert_eq!(gates.gates.len(), 2); // consumes design + produces implementation
    let status = c.status("proj").await.unwrap();
    assert_eq!((status.counter_size, status.archive_size), (1, 1));
    assert!(!status.terminated); // Counter 非空 → Rule-T 不成立

    // —— 六项静态分析（验证 `{d}:analyze` 混合段路由）——
    let report = c.analyze_domain("proj").await.unwrap();
    assert!(report.reachability.unreachable.is_empty());
    assert!(report.structural_deadlock.is_empty());

    // —— 组合装载 ——
    let merged = c.compose("composed", vec![DECL.to_owned()]).await.unwrap();
    assert_eq!(merged.types, 3);

    // —— 卸载（204）与 404 错误映射 ——
    c.unload_domain("composed").await.unwrap();
    let err = c.domain_info("composed").await.unwrap_err();
    match err {
        ClientError::Api(d) => assert_eq!(d.code, "DomainNotFound"),
        other => panic!("期望 Api 错误：{other:?}"),
    }
}

#[tokio::test]
async fn rule_rejections_map_to_structured_errors() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, _engine) = spawn_daemon(dir.path(), BTreeMap::new()).await;
    let c = Client::unix(sock);
    c.load_domain("proj", DECL).await.unwrap();

    // SeedAuthorityDenied（409）
    let err = c.seed("proj", "developer", "requirement", None).await.unwrap_err();
    match err {
        ClientError::Api(d) => assert_eq!(d.code, "SeedAuthorityDenied"),
        other => panic!("期望 Api 错误：{other:?}"),
    }

    // PreconditionMismatch（409）：produce design 但无前驱
    let err = c.produce("proj", "architect", "design", vec![], None).await.unwrap_err();
    match err {
        ClientError::Api(d) => assert_eq!(d.code, "PreconditionMismatch"),
        other => panic!("期望 Api 错误：{other:?}"),
    }

    // DomainNotFound（404）
    let err = c.seed("ghost", "external", "requirement", None).await.unwrap_err();
    match err {
        ClientError::Api(d) => assert_eq!(d.code, "DomainNotFound"),
        other => panic!("期望 Api 错误：{other:?}"),
    }

    // Syntax（400）：坏声明
    let err = c.load_domain("bad", "types {").await.unwrap_err();
    match err {
        ClientError::Api(d) => assert_eq!(d.code, "Syntax"),
        other => panic!("期望 Api 错误：{other:?}"),
    }
}

#[tokio::test]
async fn sse_events_stream_notifications() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, _engine) = spawn_daemon(dir.path(), BTreeMap::new()).await;
    let c = Client::unix(sock);
    c.load_domain("proj", DECL).await.unwrap();

    let mut events = c.events("proj").await.unwrap();
    let seed = c.seed("proj", "external", "requirement", None).await.unwrap();

    let n = tokio::time::timeout(Duration::from_secs(5), events.next_event())
        .await
        .expect("SSE 超时")
        .unwrap()
        .expect("流意外关闭");
    assert_eq!(n.event, pact_engine::NotificationKind::Seeded);
    assert_eq!(n.domain, "proj");
    assert_eq!(n.token_id, Some(seed.token.id.clone()));
    assert_eq!(n.summary.counter_size, 1);
}

#[tokio::test]
async fn auth_tokens_gate_participant_endpoints() {
    let dir = tempfile::tempdir().unwrap();
    let mut tokens = BTreeMap::new();
    tokens.insert("external".to_owned(), "sekret".to_owned());
    let (sock, _engine) = spawn_daemon(dir.path(), tokens).await;

    // 无令牌 → 401
    let anon = Client::unix(sock.clone());
    anon.load_domain("proj", DECL).await.unwrap(); // 管理端点不受令牌约束
    let err = anon.seed("proj", "external", "requirement", None).await.unwrap_err();
    match err {
        ClientError::Api(d) => assert_eq!(d.code, "Unauthorized"),
        other => panic!("期望 Api 错误：{other:?}"),
    }
    // 错令牌 → 401
    let wrong = Client::unix(sock.clone()).with_bearer("wrong");
    let err = wrong.seed("proj", "external", "requirement", None).await.unwrap_err();
    assert!(matches!(err, ClientError::Api(d) if d.code == "Unauthorized"));
    // 正确令牌 → 放行
    let ok = Client::unix(sock).with_bearer("sekret");
    let resp = ok.seed("proj", "external", "requirement", None).await.unwrap();
    assert_eq!(resp.token.ty.0, "requirement");
}

#[tokio::test]
async fn tcp_listener_equivalent() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("data")).await.unwrap();
    let engine = Engine::spawn(
        store,
        Arc::new(LogicalClock::new()),
        Arc::new(SequentialIdGen::new()),
    );
    let state = ServerState::new(engine, BTreeMap::new());
    let app = router(state);
    // 端口 0：由系统分配，但本工具函数只接受字符串地址；用固定高位端口
    let addr = "127.0.0.1:18791";
    tokio::spawn(async move {
        let _ = pact_http::server::serve_tcp(app, addr).await;
    });
    // 等监听就绪
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let c = Client::tcp(addr);
    c.load_domain("proj", DECL).await.unwrap();
    let status = c.status("proj").await.unwrap();
    assert_eq!(status.counter_size, 0);
}
