//! pact-http 端到端：真实 daemon（unix socket）+ 类型化 client 全生命周期。
//!
//! 覆盖 §6 线上格式、§9.2 端点、错误码映射、§6.4 SSE、§9.4 令牌校验。
//! 域双键（D27）：域级端点作用于 `default` 实例；实例级端点走 `/instances/{i}`。

use pact_engine::{Engine, LogicalClock, SequentialIdGen};
use pact_http::client::{Client, ClientError};
use pact_http::server::{router, serve_unix, ConnTracker, ServerState};
use pact_store::Store;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
        let _ = serve_unix(app, &s, watch::channel(false).1, ConnTracker::default()).await;
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
    assert_eq!(
        (
            v.types.len(),
            v.participants.len(),
            v.gates,
            v.seed_authority
        ),
        (3, 4, 6, 1)
    );
    // 非法声明 → §6.3 结构化错误（不加载）
    match c.validate("types { a }").await {
        Err(ClientError::Api(d)) => assert!(matches!(d.code.as_str(), "WellFormedness" | "Syntax")),
        other => panic!("预期良构违例，实际 {other:?}"),
    }

    let summary = c.load_domain("proj", DECL, None, false).await.unwrap();
    assert_eq!(summary.name, "proj");
    assert_eq!(summary.instance, "default");
    assert_eq!(
        (summary.types, summary.gates, summary.seed_authority),
        (3, 6, 1)
    );

    let list = c.list_domains().await.unwrap();
    assert_eq!(list.domains.len(), 1);
    assert_eq!(list.domains[0].name, "proj");
    assert_eq!(list.domains[0].instances, vec!["default".to_owned()]);

    let info = c.domain_info("proj", None).await.unwrap();
    assert_eq!(info.counter_size, 0);
    // Rule-T 信息性判定：Counter 空且无可执行 Produce → 成立（种子仍可注入）
    assert!(info.terminated);

    // —— Rule-S ——
    let seed = c
        .seed("proj", None, "external", "requirement", Some("REQ-1"))
        .await
        .unwrap();
    assert_eq!(seed.token.ty.0, "requirement");
    let req_id = seed.token.id.0.clone();

    // —— enabled（§6.5，字段名 type）——
    let enabled = c.enabled("proj", None, "architect").await.unwrap();
    assert_eq!(enabled.options.len(), 1);
    assert_eq!(enabled.options[0].ty.0, "design");
    assert_eq!(enabled.options[0].witness_predecessors[0].0, req_id);

    // —— Rule-P ——
    let prod = c
        .produce(
            "proj",
            None,
            "architect",
            "design",
            vec![req_id.clone()],
            None,
        )
        .await
        .unwrap();
    let design_id = prod.token.id.0.clone();

    // —— Rule-R ——
    let read = c
        .read_token("proj", None, "architect", &req_id)
        .await
        .unwrap();
    assert_eq!(read.token.id.0, req_id);

    // —— Rule-E（带幂等键；重试返回首次结果）——
    let entry = c
        .consume("proj", None, "developer", &design_id, Some("api-consume-1"))
        .await
        .unwrap();
    assert_eq!(entry.entry.token.id.0, design_id);
    assert_eq!(entry.entry.exhausted_by.0, "developer");
    let retry = c
        .consume("proj", None, "developer", &design_id, Some("api-consume-1"))
        .await
        .unwrap();
    assert_eq!(retry.entry, entry.entry, "重试返回首次归档结果");

    // —— 查询 ——
    let counter = c.counter("proj", None, None).await.unwrap();
    assert_eq!(counter.size, 1); // 仅剩 requirement
    let counter_f = c.counter("proj", None, Some("design")).await.unwrap();
    assert_eq!(counter_f.size, 0);
    let archive = c.archive("proj", None, None).await.unwrap();
    assert_eq!(archive.size, 1);
    let gates = c.gates("proj", None, "developer").await.unwrap();
    assert_eq!(gates.gates.len(), 2); // consumes design + produces implementation
    let status = c.status("proj", None).await.unwrap();
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
    c.unload_domain("composed", None).await.unwrap();
    let err = c.domain_info("composed", None).await.unwrap_err();
    match err {
        ClientError::Api(d) => assert_eq!(d.code, "InstanceNotFound"),
        other => panic!("期望 Api 错误：{other:?}"),
    }
}

#[tokio::test]
async fn rule_rejections_map_to_structured_errors() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, _engine) = spawn_daemon(dir.path(), BTreeMap::new()).await;
    let c = Client::unix(sock);
    c.load_domain("proj", DECL, None, false).await.unwrap();

    // SeedAuthorityDenied（409）
    let err = c
        .seed("proj", None, "developer", "requirement", None)
        .await
        .unwrap_err();
    match err {
        ClientError::Api(d) => assert_eq!(d.code, "SeedAuthorityDenied"),
        other => panic!("期望 Api 错误：{other:?}"),
    }

    // PreconditionMismatch（409）：produce design 但无前驱
    let err = c
        .produce("proj", None, "architect", "design", vec![], None)
        .await
        .unwrap_err();
    match err {
        ClientError::Api(d) => assert_eq!(d.code, "PreconditionMismatch"),
        other => panic!("期望 Api 错误：{other:?}"),
    }

    // InstanceNotFound（404）：未加载的实例
    let err = c
        .seed("ghost", None, "external", "requirement", None)
        .await
        .unwrap_err();
    match err {
        ClientError::Api(d) => assert_eq!(d.code, "InstanceNotFound"),
        other => panic!("期望 Api 错误：{other:?}"),
    }

    // Syntax（400）：坏声明
    let err = c
        .load_domain("bad", "types {", None, false)
        .await
        .unwrap_err();
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
    c.load_domain("proj", DECL, None, false).await.unwrap();

    let mut events = c.events("proj", None).await.unwrap();
    let seed = c
        .seed("proj", None, "external", "requirement", None)
        .await
        .unwrap();

    let n = tokio::time::timeout(Duration::from_secs(5), events.next_event())
        .await
        .expect("SSE 超时")
        .unwrap()
        .expect("流意外关闭");
    assert_eq!(n.event, pact_engine::NotificationKind::Seeded);
    assert_eq!(n.domain, "proj");
    assert_eq!(n.instance, "default");
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
    anon.load_domain("proj", DECL, None, false).await.unwrap(); // 管理端点不受令牌约束
    let err = anon
        .seed("proj", None, "external", "requirement", None)
        .await
        .unwrap_err();
    match err {
        ClientError::Api(d) => assert_eq!(d.code, "Unauthorized"),
        other => panic!("期望 Api 错误：{other:?}"),
    }
    // 错令牌 → 401
    let wrong = Client::unix(sock.clone()).with_bearer("wrong");
    let err = wrong
        .seed("proj", None, "external", "requirement", None)
        .await
        .unwrap_err();
    assert!(matches!(err, ClientError::Api(d) if d.code == "Unauthorized"));
    // 正确令牌 → 放行
    let ok = Client::unix(sock).with_bearer("sekret");
    let resp = ok
        .seed("proj", None, "external", "requirement", None)
        .await
        .unwrap();
    assert_eq!(resp.token.ty.0, "requirement");

    // 实例级端点受同一 authorize 通径约束（D27 镜像组）
    anon.instantiate("proj", "x").await.unwrap();
    let err = anon
        .seed("proj", Some("x"), "external", "requirement", None)
        .await
        .unwrap_err();
    match err {
        ClientError::Api(d) => assert_eq!(d.code, "Unauthorized"),
        other => panic!("期望 Api 错误：{other:?}"),
    }
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
        let _ = pact_http::server::serve_tcp(
            app,
            addr,
            watch::channel(false).1,
            ConnTracker::default(),
        )
        .await;
    });
    // 等监听就绪
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let c = Client::tcp(addr);
    c.load_domain("proj", DECL, None, false).await.unwrap();
    let status = c.status("proj", None).await.unwrap();
    assert_eq!(status.counter_size, 0);
}

// ============ 域双键：实例级路由镜像（D27） ============

#[tokio::test]
async fn instance_routes_are_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, _engine) = spawn_daemon(dir.path(), BTreeMap::new()).await;
    let c = Client::unix(sock);

    // default 实例 + 显式新增实例 x
    c.load_domain("proj", DECL, None, false).await.unwrap();
    let summary = c.instantiate("proj", "x").await.unwrap();
    assert_eq!(summary.instance, "x");

    // 清单含两个实例
    let list = c.list_domains().await.unwrap();
    assert_eq!(list.domains[0].instances, vec!["default", "x"]);

    // default 注入 1 枚、x 注入 2 枚，互不可见
    c.seed("proj", None, "external", "requirement", None)
        .await
        .unwrap();
    c.seed("proj", Some("x"), "external", "requirement", None)
        .await
        .unwrap();
    c.seed("proj", Some("x"), "external", "requirement", None)
        .await
        .unwrap();
    assert_eq!(c.counter("proj", None, None).await.unwrap().size, 1);
    assert_eq!(c.counter("proj", Some("x"), None).await.unwrap().size, 2);

    // 重复实例化 → InstanceExists（409）
    let err = c.instantiate("proj", "x").await.unwrap_err();
    match err {
        ClientError::Api(d) => assert_eq!(d.code, "InstanceExists"),
        other => panic!("期望 Api 错误：{other:?}"),
    }

    // 非空实例再 load 无 force → DomainBusy（409）
    let err = c
        .load_domain("proj", DECL, Some("x"), false)
        .await
        .unwrap_err();
    match err {
        ClientError::Api(d) => assert_eq!(d.code, "DomainBusy"),
        other => panic!("期望 Api 错误：{other:?}"),
    }

    // 实例级卸载不影响 default 实例
    c.unload_domain("proj", Some("x")).await.unwrap();
    assert_eq!(c.counter("proj", None, None).await.unwrap().size, 1);
}

// ============ 安全边界：键名非法（含路径穿越）→ 400，且不落盘 ============

#[tokio::test]
async fn invalid_key_names_rejected_without_touching_fs() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, _engine) = spawn_daemon(dir.path(), BTreeMap::new()).await;
    let c = Client::unix(sock);
    c.load_domain("proj", DECL, None, false).await.unwrap();

    // 实例名含路径分隔符 / `..`（经请求体注入）→ 400 InvalidInstanceName
    for evil in ["../../escape", "a/b", "..", "x.y", ""] {
        let err = c.instantiate("proj", evil).await.unwrap_err();
        match err {
            ClientError::Api(d) => assert_eq!(d.code, "InvalidInstanceName", "实例名 {evil:?}"),
            other => panic!("实例名 {evil:?} 期望 Api 错误：{other:?}"),
        }
    }
    // 域子目录外无越界文件产生
    assert!(!dir.path().join("escape.events.jsonl").exists());
    assert!(!dir.path().join("data").join("escape").exists());

    // 非法域名（经请求体注入）→ 400 InvalidDomainName
    for evil in ["../evil", "a b", "d.e", ""] {
        let err = c.load_domain(evil, DECL, None, false).await.unwrap_err();
        match err {
            ClientError::Api(d) => assert_eq!(d.code, "InvalidDomainName", "域名 {evil:?}"),
            other => panic!("域名 {evil:?} 期望 Api 错误：{other:?}"),
        }
    }
    assert!(!dir.path().join("data").join("evil").exists());
}

// ============ 运维端点（D25 子项 2：health / ready / metrics） ============

/// 裸 HTTP/1.1 GET（unix socket），返回 (状态码, 响应体)。
async fn raw_get(sock: &Path, path: &str) -> (u16, String) {
    let mut stream = tokio::net::UnixStream::connect(sock).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = String::new();
    stream.read_to_string(&mut buf).await.unwrap();
    let status = buf
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = buf
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_owned())
        .unwrap_or_default();
    (status, body)
}

/// 运维端点不校验令牌：即便配了令牌表，health/ready/metrics 仍免认证放行。
#[tokio::test]
async fn ops_endpoints_bypass_auth() {
    let dir = tempfile::tempdir().unwrap();
    let mut tokens = BTreeMap::new();
    tokens.insert("external".to_owned(), "sekret".to_owned());
    let (sock, _engine) = spawn_daemon(dir.path(), tokens).await;

    let (s, b) = raw_get(&sock, "/health").await;
    assert_eq!(s, 200);
    assert!(b.contains("\"status\":\"ok\""), "body={b}");

    let (s, b) = raw_get(&sock, "/ready").await;
    assert_eq!(s, 200);
    assert!(b.contains("\"status\":\"ready\""), "body={b}");

    let (s, b) = raw_get(&sock, "/metrics").await;
    assert_eq!(s, 200);
    assert!(b.contains("pact_uptime_seconds"), "body={b}");
    assert!(b.contains("pact_instances_loaded 0"), "body={b}");
}

/// 指标反映已加载域实例的 counter/archive/terminated。
#[tokio::test]
async fn metrics_reports_loaded_instance() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, _engine) = spawn_daemon(dir.path(), BTreeMap::new()).await;
    let c = Client::unix(sock.clone());
    c.load_domain("proj", DECL, None, false).await.unwrap();

    let (s, b) = raw_get(&sock, "/metrics").await;
    assert_eq!(s, 200);
    assert!(b.contains("pact_domains_declared 1"), "body={b}");
    assert!(b.contains("pact_instances_loaded 1"), "body={b}");
    assert!(
        b.contains("pact_counter_size{domain=\"proj\",instance=\"default\"} 0"),
        "body={b}"
    );
    assert!(
        b.contains("pact_terminated{domain=\"proj\",instance=\"default\"} 1"),
        "body={b}"
    );
}

// ============ 日志压缩与 schema 版本化（D25 子项 1） ============

/// `default` 实例的事件日志路径。
fn log_path(dir: &Path, domain: &str) -> PathBuf {
    dir.join("data").join(domain).join("default.events.jsonl")
}

fn log_lines(dir: &Path, domain: &str) -> Vec<String> {
    std::fs::read_to_string(log_path(dir, domain))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// `POST .../compact`：截断已入快照的前缀、状态与幂等表经快照存活、重启后续接不重号。
#[tokio::test]
async fn compact_endpoint_truncates_log_and_preserves_state() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, engine) = spawn_daemon(dir.path(), BTreeMap::new()).await;
    let c = Client::unix(sock);

    c.load_domain("proj", DECL, None, false).await.unwrap();
    let req = c
        .seed("proj", None, "external", "requirement", Some("REQ-1"))
        .await
        .unwrap();
    let design = c
        .produce(
            "proj",
            None,
            "architect",
            "design",
            vec![req.token.id.0.clone()],
            Some("DES-1"),
        )
        .await
        .unwrap();
    let first = c
        .consume("proj", None, "architect", &req.token.id.0, Some("req-1"))
        .await
        .unwrap();
    let _ = design;

    // 线上格式带顶层 version（schema 版本化）
    let before = log_lines(dir.path(), "proj");
    assert_eq!(before.len(), 4, "DomainLoaded/Seeded/Produced/Consumed");
    assert!(before.iter().all(|l| l.contains(r#""version":1"#)));
    let status = c.status("proj", None).await.unwrap();
    assert_eq!((status.counter_size, status.archive_size), (1, 1));

    // 压缩（keep_last = 0）：日志清空，返回水位与丢弃计数
    let out = c.compact("proj", None, 0).await.unwrap();
    assert_eq!(
        (out.domain.as_str(), out.instance.as_str()),
        ("proj", "default")
    );
    assert_eq!((out.snapshot_seq, out.base_seq), (4, 4));
    assert_eq!((out.retained_events, out.discarded_events), (0, 4));
    assert!(log_lines(dir.path(), "proj").is_empty());

    // 状态不变；重复压缩水位不回退
    let status = c.status("proj", None).await.unwrap();
    assert_eq!((status.counter_size, status.archive_size), (1, 1));
    let again = c.compact("proj", None, 0).await.unwrap();
    assert_eq!((again.base_seq, again.discarded_events), (4, 0));

    // 新事件自 seq=5 续接
    c.seed("proj", None, "external", "requirement", Some("REQ-2"))
        .await
        .unwrap();
    let tail = log_lines(dir.path(), "proj");
    assert_eq!(tail.len(), 1);
    assert!(tail[0].contains(r#""seq":5"#), "tail={tail:?}");

    // 重启：日志无 DomainLoaded 行，靠快照锚恢复状态与幂等表
    engine.shutdown().await.unwrap();
    let (sock2, engine2) = spawn_daemon(dir.path(), BTreeMap::new()).await;
    let c2 = Client::unix(sock2);
    let status = c2.status("proj", None).await.unwrap();
    assert_eq!((status.counter_size, status.archive_size), (2, 1));
    let retry = c2
        .consume("proj", None, "architect", &req.token.id.0, Some("req-1"))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&retry.entry).unwrap(),
        serde_json::to_value(&first.entry).unwrap(),
        "压缩 + 重启后幂等表仍生效（快照承载）"
    );
    assert_eq!(log_lines(dir.path(), "proj").len(), 1, "幂等重试不追加事件");
    engine2.shutdown().await.unwrap();
}

/// 压缩不存在的实例 → 404 InstanceNotFound（不静默成功）。
#[tokio::test]
async fn compact_unknown_instance_returns_404() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, _engine) = spawn_daemon(dir.path(), BTreeMap::new()).await;
    let c = Client::unix(sock);
    c.load_domain("proj", DECL, None, false).await.unwrap();
    let err = c.compact("proj", Some("nope"), 0).await.unwrap_err();
    match err {
        ClientError::Api(d) => assert_eq!(d.code, "InstanceNotFound"),
        other => panic!("期望 Api 错误：{other:?}"),
    }
}

// ============ 优雅停机（D25 子项 2） ============

/// 停机信号置位后监听循环退出（不再接受新连接），在途连接归零。
#[tokio::test]
async fn graceful_shutdown_stops_accepting_and_drains() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("data")).await.unwrap();
    let engine = Engine::spawn(
        store,
        Arc::new(LogicalClock::new()),
        Arc::new(SequentialIdGen::new()),
    );
    let app = router(ServerState::new(engine, BTreeMap::new()));

    let sock = dir.path().join("pact.sock");
    let (tx, rx) = watch::channel(false);
    let tracker = ConnTracker::default();
    let s = sock.clone();
    let tk = tracker.clone();
    let handle = tokio::spawn(async move { serve_unix(app, &s, rx, tk).await });

    // 等待监听就绪（一次成功连接即可）
    let mut ready = false;
    for _ in 0..100 {
        if tokio::net::UnixStream::connect(&sock).await.is_ok() {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(ready, "监听未就绪");

    // 广播停机 → 监听循环应返回
    tx.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("监听循环应在停机信号后退出")
        .unwrap()
        .unwrap();

    // 监听已关闭：新连接应被拒绝
    assert!(
        tokio::net::UnixStream::connect(&sock).await.is_err(),
        "停机后不应再接受新连接"
    );
    // 在途连接归零
    tokio::time::timeout(Duration::from_secs(5), tracker.wait_idle())
        .await
        .expect("在途连接应归零");
    assert_eq!(tracker.active(), 0);
}
