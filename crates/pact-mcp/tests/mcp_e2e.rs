//! pact-mcp 端到端：真实 `pact-mcp` 子进程（stdio，经 REST 接入进程内起的真实
//! daemon）+ rmcp client 走完整 PACT 生命周期（§9.1 工具集）。

use pact_engine::{Engine, LogicalClock, SequentialIdGen};
use pact_http::server::{router, serve_unix, ConnTracker, ServerState};
use pact_store::Store;
use rmcp::model::{CallToolRequestParams, ContentBlock};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::ServiceExt;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
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

/// 换版声明：新增 `review` 类型与门。
const DECL_V2: &str = r#"
types { requirement; design; implementation; review }

preconditions {
    requirement → design;
    design → implementation;
    design → review
}

gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: reads design, consumes design, produces implementation;
    reviewer: consumes design, produces review;
    user: consumes implementation
}

seed_authority { external: seeds requirement }
"#;

/// 进程内起一个真实 daemon（临时目录 + unix socket）。
async fn spawn_daemon(dir: &Path) -> PathBuf {
    let store = Store::open(dir.join("data")).await.unwrap();
    let engine = Engine::spawn(
        store,
        Arc::new(LogicalClock::new()),
        Arc::new(SequentialIdGen::new()),
    );
    let state = ServerState::new(engine, BTreeMap::new());
    let app = router(state);
    let sock = dir.join("pact.sock");
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
    panic!("daemon 套接字未就绪");
}

/// 调工具并解析 §6 线上格式文本内容为 JSON。
async fn tool_ok(client: &rmcp::Peer<rmcp::RoleClient>, name: &str, args: Value) -> Value {
    let params = CallToolRequestParams::new(name.to_owned())
        .with_arguments(serde_json::from_value(args).unwrap());
    let res = client.call_tool(params).await.unwrap();
    assert!(
        !res.is_error.unwrap_or(false),
        "工具 {name} 不应报错: {res:?}"
    );
    let ContentBlock::Text(t) = &res.content[0] else {
        panic!("工具 {name} 返回非文本内容")
    };
    serde_json::from_str(&t.text).expect("工具输出应为合法 JSON")
}

/// 调工具并断言以 §6.3 错误体失败。
async fn tool_err(client: &rmcp::Peer<rmcp::RoleClient>, name: &str, args: Value) -> Value {
    let params = CallToolRequestParams::new(name.to_owned())
        .with_arguments(serde_json::from_value(args).unwrap());
    let res = client.call_tool(params).await.unwrap();
    assert_eq!(res.is_error, Some(true), "工具 {name} 应报错");
    let ContentBlock::Text(t) = &res.content[0] else {
        panic!("工具 {name} 返回非文本内容")
    };
    serde_json::from_str(&t.text).expect("错误输出应为 §6.3 错误体 JSON")
}

#[tokio::test]
async fn mcp_full_lifecycle_over_stdio() {
    let dir = tempfile::tempdir().unwrap();
    let sock = spawn_daemon(dir.path()).await;

    // 起真实 pact-mcp 子进程（stdio）
    let (transport, _stderr) = TokioChildProcess::builder(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_pact-mcp")).configure(|c| {
            c.arg("--unix").arg(&sock);
        }),
    )
    .spawn()
    .expect("启动 pact-mcp 子进程失败");
    let client = ().serve(transport).await.expect("MCP 握手失败");

    // —— 工具清单（§9.1 + D27 + D25 + D39：21 个） ——
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 21, "应注册 21 个工具: {tools:?}");

    // —— 域生命周期 ——
    let v = tool_ok(
        &client,
        "pact_domain_validate",
        json!({ "declaration": DECL }),
    )
    .await;
    assert_eq!(v["ok"], true);

    let bad = tool_err(
        &client,
        "pact_domain_validate",
        json!({ "declaration": "types { a }" }),
    )
    .await;
    assert!(["WellFormedness", "Syntax"].contains(&bad["error"]["code"].as_str().unwrap()));

    let summary = tool_ok(
        &client,
        "pact_domain_load",
        json!({ "name": "proj", "declaration": DECL }),
    )
    .await;
    assert_eq!(summary["name"], "proj");
    assert_eq!(summary["types"], 3);

    let list = tool_ok(&client, "pact_list_domains", json!({})).await;
    assert_eq!(list["domains"][0]["name"], "proj");
    assert_eq!(list["domains"][0]["instances"], json!(["default"]));

    // —— 域实例化（D27）：新增实例 x，实例级工具透传 instance ——
    let inst = tool_ok(
        &client,
        "pact_domain_instantiate",
        json!({ "domain": "proj", "instance": "x" }),
    )
    .await;
    assert_eq!(inst["instance"], "x");
    let seeded_x = tool_ok(
        &client,
        "pact_seed",
        json!({ "domain": "proj", "instance": "x", "participant": "external", "type": "requirement" }),
    )
    .await;
    assert_eq!(seeded_x["token"]["type"], "requirement");
    let counter_x = tool_ok(
        &client,
        "pact_counter_query",
        json!({ "domain": "proj", "instance": "x" }),
    )
    .await;
    assert_eq!(counter_x["size"], 1);
    let counter_default = tool_ok(&client, "pact_counter_query", json!({ "domain": "proj" })).await;
    assert_eq!(counter_default["size"], 0, "default 实例不受 x 影响");

    // —— Rule-S：越权 → 错误体；授权参与者 → 成功 ——
    let denied = tool_err(
        &client,
        "pact_seed",
        json!({ "domain": "proj", "participant": "architect", "type": "requirement" }),
    )
    .await;
    assert_eq!(denied["error"]["code"], "SeedAuthorityDenied");
    let seed = tool_ok(
        &client,
        "pact_seed",
        json!({ "domain": "proj", "participant": "external", "type": "requirement", "thing_ref": "REQ-1" }),
    )
    .await;
    let req_id = seed["token"]["id"].as_str().unwrap().to_owned();

    // —— Rule-W ——
    let enabled = tool_ok(
        &client,
        "pact_enabled",
        json!({ "domain": "proj", "participant": "architect" }),
    )
    .await;
    assert_eq!(enabled["options"][0]["type"], "design");

    // —— Rule-P（前驱不匹配 → 错误；正确 → 成功） ——
    let mismatch = tool_err(
        &client,
        "pact_produce",
        json!({ "domain": "proj", "participant": "architect", "type": "design", "predecessors": [] }),
    )
    .await;
    assert_eq!(mismatch["error"]["code"], "PreconditionMismatch");
    let prod = tool_ok(
        &client,
        "pact_produce",
        json!({
            "domain": "proj", "participant": "architect", "type": "design",
            "predecessors": [req_id]
        }),
    )
    .await;
    let design_id = prod["token"]["id"].as_str().unwrap().to_owned();

    // —— Rule-R ——
    let read = tool_ok(
        &client,
        "pact_read",
        json!({ "domain": "proj", "participant": "architect", "token_id": req_id }),
    )
    .await;
    assert_eq!(read["token"]["id"], req_id);

    // —— Rule-E ——
    let entry = tool_ok(
        &client,
        "pact_consume",
        json!({ "domain": "proj", "participant": "developer", "token_id": design_id }),
    )
    .await;
    assert_eq!(entry["entry"]["token"]["id"], design_id);
    assert_eq!(entry["entry"]["exhausted_by"], "developer");

    // —— 查询与分析 ——
    let counter = tool_ok(&client, "pact_counter_query", json!({ "domain": "proj" })).await;
    assert_eq!(counter["size"], 1);
    let archive = tool_ok(
        &client,
        "pact_archive_query",
        json!({ "domain": "proj", "type": "design" }),
    )
    .await;
    assert_eq!(archive["size"], 1);
    let gates = tool_ok(
        &client,
        "pact_my_gates",
        json!({ "domain": "proj", "participant": "developer" }),
    )
    .await;
    assert_eq!(gates["gates"].as_array().unwrap().len(), 2);
    let report = tool_ok(&client, "pact_analyze", json!({ "domain": "proj" })).await;
    assert!(report["reachability"]["unreachable"]
        .as_array()
        .unwrap()
        .is_empty());
    // A.5 报告（rev2：三元结构 Violations / Warnings / WF(D)）
    assert_eq!(report["precondition_wf"]["well_formed"], true);
    assert!(report["precondition_wf"]["violations"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(report["precondition_wf"]["warnings"]
        .as_array()
        .unwrap()
        .is_empty());
    let status = tool_ok(&client, "pact_status", json!({ "domain": "proj" })).await;
    assert_eq!(status["counter_size"], 1);
    assert_eq!(status["archive_size"], 1);

    // —— 日志压缩（D25 子项 1）：截断已入快照的前缀，状态经快照保持 ——
    let compacted = tool_ok(
        &client,
        "pact_compact",
        json!({ "domain": "proj", "keep_last": 0 }),
    )
    .await;
    assert_eq!(compacted["instance"], "default");
    assert_eq!(compacted["base_seq"], 4);
    assert_eq!(compacted["discarded_events"], 4);
    let after = tool_ok(&client, "pact_status", json!({ "domain": "proj" })).await;
    assert_eq!(after["counter_size"], 1);
    assert_eq!(after["archive_size"], 1);

    client.cancel().await.unwrap();
}

#[tokio::test]
async fn mcp_state_handoff_tools_over_stdio() {
    let dir = tempfile::tempdir().unwrap();
    let sock = spawn_daemon(dir.path()).await;

    let (transport, _stderr) = TokioChildProcess::builder(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_pact-mcp")).configure(|c| {
            c.arg("--unix").arg(&sock);
        }),
    )
    .spawn()
    .expect("启动 pact-mcp 子进程失败");
    let client = ().serve(transport).await.expect("MCP 握手失败");

    tool_ok(
        &client,
        "pact_domain_load",
        json!({ "name": "proj", "declaration": DECL }),
    )
    .await;
    let seed = tool_ok(
        &client,
        "pact_seed",
        json!({ "domain": "proj", "participant": "external", "type": "requirement" }),
    )
    .await;
    let req_id = seed["token"]["id"].as_str().unwrap().to_owned();
    tool_ok(
        &client,
        "pact_produce",
        json!({
            "domain": "proj", "participant": "architect", "type": "design",
            "predecessors": [req_id]
        }),
    )
    .await;

    // 交接：源 default → 目标 v2，默认激活。
    let out = tool_ok(
        &client,
        "pact_domain_handoff",
        json!({
            "domain": "proj",
            "declaration": DECL_V2,
            "source_instance": "default",
            "target_instance": "v2"
        }),
    )
    .await;
    assert_eq!(out["counter"], 2);
    assert_eq!(out["archive"], 0);
    assert_eq!(out["active_instance"], "v2");
    assert_eq!(out["activated"], true);

    let active = tool_ok(&client, "pact_domain_active", json!({ "domain": "proj" })).await;
    assert_eq!(active["instance"], "v2");

    // 短路由工具（不传 instance）解析到 v2。
    let short = tool_ok(&client, "pact_counter_query", json!({ "domain": "proj" })).await;
    assert_eq!(short["size"], 2);

    // 源实例已冻结：显式实例调用被拒。
    let denied = tool_err(
        &client,
        "pact_seed",
        json!({ "domain": "proj", "instance": "default", "participant": "external", "type": "requirement" }),
    )
    .await;
    assert_eq!(denied["error"]["code"], "InstanceFrozen");

    // 冻结工具：解冻后源恢复可写，再冻结立即生效。
    let unfrozen = tool_ok(
        &client,
        "pact_domain_freeze",
        json!({ "domain": "proj", "instance": "default", "unfreeze": true }),
    )
    .await;
    assert_eq!(unfrozen["frozen"], false);
    tool_ok(
        &client,
        "pact_seed",
        json!({ "domain": "proj", "instance": "default", "participant": "external", "type": "requirement" }),
    )
    .await;
    tool_ok(
        &client,
        "pact_domain_freeze",
        json!({ "domain": "proj", "instance": "default" }),
    )
    .await;
    let denied = tool_err(
        &client,
        "pact_seed",
        json!({ "domain": "proj", "instance": "default", "participant": "external", "type": "requirement" }),
    )
    .await;
    assert_eq!(denied["error"]["code"], "InstanceFrozen");

    // 切换活动实例。
    let switched = tool_ok(
        &client,
        "pact_domain_activate",
        json!({ "domain": "proj", "instance": "default" }),
    )
    .await;
    assert_eq!(switched["instance"], "default");
    assert_eq!(
        tool_ok(&client, "pact_domain_active", json!({ "domain": "proj" })).await["instance"],
        "default"
    );

    client.cancel().await.unwrap();
}
