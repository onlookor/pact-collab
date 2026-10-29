//! pact-runtime 集成测试（阶段 7）。
//!
//! 全部场景在**单个多线程 `#[tokio::test]` 内顺序执行**——tokio 的多个测试各自建 runtime，
//! 与派生任务/子进程的回收存在陷阱（同 `pact-mcp` 的 `unattended_e2e` 注释）。集成测试采用
//! **in-process daemon + REST 传输**；MCP（stdio 子进程）路径与本仓 e2e 同构，由 bin/生产使用。

use pact_core::ArchiveEntry;
use pact_engine::notify::Notification;
use pact_engine::views::StatusView;
use pact_engine::{Engine, LogicalClock, SequentialIdGen};
use pact_http::client::Client;
use pact_http::dto::ErrorDetail;
use pact_http::server::{router, serve_unix, ConnTracker, ServerState};
use pact_http::visibility::VisibilityProfiles;
use pact_runtime::driver::absorb;
use pact_runtime::{
    AgentReport, AgentSpec, Bootstrap, Compensation, Endpoint, ExecutorConfig, Orchestrator,
    RestClient, RuntimeError, SideEffectExecutor, SideEffectHandler, StopSignal,
};
use pact_store::Store;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

// ============================================================
// 域声明
// ============================================================

/// 论文 Part II §1.5：软件工程生命周期域（D17 修订：external_stakeholder 为种子权威）。
const LIFECYCLE: &str = r#"
types { requirement; design; implementation; test_report; deployment }

preconditions {
    requirement → design;
    design → implementation;
    implementation → test_report;
    test_report → deployment
}

gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: reads design, consumes design, produces implementation;
    tester: reads implementation, consumes implementation, produces test_report;
    operator: reads test_report, consumes test_report, produces deployment;
    stakeholder: consumes deployment
}

seed_authority { external_stakeholder: seeds requirement }
"#;

/// 论文 Part III Ch.1：ReAct 循环域。
const REACT: &str = r#"
types { observation; thought; action; tool_result }

preconditions {
    observation → thought;
    thought → action;
    action → tool_result;
    tool_result → observation
}

gates {
    agent: reads observation, consumes observation, produces thought,
            reads thought, consumes thought, produces action,
            reads action, consumes action, produces tool_result,
            reads tool_result, consumes tool_result, produces observation
}

seed_authority { user: seeds observation }
"#;

/// 外部物化确认域（saga 路径）：`task` 消费即触发外部动作；
/// `task_failure` 为源类型，唯一入口是 materializer 的种子权（补偿事实）。
const MATERIALIZATION: &str = r#"
types { task; artifact; task_failure }

preconditions { task → artifact }

gates {
    worker: reads task, consumes task, produces artifact;
    sink: consumes artifact;
    materializer: consumes task_failure
}

seed_authority {
    seeder: seeds task;
    materializer: seeds task_failure
}
"#;

// ============================================================
// 测试设施
// ============================================================

/// in-process daemon（镜像 `unattended_e2e::spawn_daemon`）。
async fn spawn_daemon(dir: &Path) -> PathBuf {
    let store = Store::open(dir.join("data")).await.unwrap();
    let engine = Engine::spawn(
        store,
        Arc::new(LogicalClock::new()),
        Arc::new(SequentialIdGen::new()),
    );
    // D42：默认 read-gate 会隐藏无主体视图的票面；本场景需要核验补偿事实的
    // `thing_ref` 关联，因此为 mat 域显式声明 domain 可见性。
    // D45：加载 RefBindings 后默认只接受 URI；本场景沿用旧 Identifier（`task-1`、
    // token ULID），须显式放行。
    let refs = dir.join("refs.yaml");
    std::fs::write(
        &refs,
        r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: mat
spec:
  defaults:
    ref:
      visibility: domain
      allowIdentifier: true
"#,
    )
    .unwrap();
    let visibility = VisibilityProfiles::load(&[refs]).unwrap();
    let state = ServerState::with_identity_and_visibility(
        engine,
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        None,
        None,
        visibility,
    );
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

/// 外部副作用处理器：对指定类型返回失败，其余成功。
struct FailingHandler {
    fail_on: String,
}

#[async_trait::async_trait]
impl SideEffectHandler for FailingHandler {
    async fn apply(&self, entry: &ArchiveEntry) -> Result<(), String> {
        if entry.token.ty.0 == self.fail_on {
            Err(format!("外部动作失败：token={}", entry.token.id.0))
        } else {
            Ok(())
        }
    }
}

/// 构造一条通知（供 [`absorb`] 单元测试）。
fn notification(kind: &str, counter_size: usize) -> Notification {
    serde_json::from_value(json!({
        "event": kind,
        "domain": "d",
        "instance": "default",
        "seq": 1,
        "ts": "2026-10-19T00:00:00Z",
        "token_id": null,
        "summary": { "counter_size": counter_size },
    }))
    .unwrap()
}

/// 构造一条归档条目（供 [`ExecutorConfig::watches`] 单元测试）。
fn archive_entry(ty: &str) -> ArchiveEntry {
    serde_json::from_value(json!({
        "token": {
            "id": "01J000000000000000000001",
            "type": ty,
            "thing_ref": null,
            "produced_by": "p",
            "depends_on": [],
            "produced_at": "2026-10-19T00:00:00Z",
        },
        "exhausted_by": "p",
        "exhausted_at": "2026-10-19T00:00:01Z",
    }))
    .unwrap()
}

// ============================================================
// T4 单元测试
// ============================================================

#[test]
fn absorb_counts_and_flags() {
    let mut rep = AgentReport::default();
    absorb(&mut rep, &notification("Terminated", 2));
    assert_eq!(rep.notifications, 1);
    assert_eq!(rep.max_counter, 2);
    assert!(rep.notified_terminated);

    absorb(&mut rep, &notification("Produced", 1));
    assert_eq!(rep.notifications, 2);
    assert_eq!(rep.max_counter, 2, "峰值不被回退");
    assert!(rep.notified_terminated, "Terminated 标记只置不消");
}

#[test]
fn executor_watches_filter() {
    let cfg = ExecutorConfig::new("m").watching(["task", "artifact"]);
    assert!(cfg.watches(&archive_entry("task")));
    assert!(cfg.watches(&archive_entry("artifact")));
    assert!(!cfg.watches(&archive_entry("task_failure")));

    let all = ExecutorConfig::new("m");
    assert!(all.watches(&archive_entry("anything")), "空集 = 全部");
}

#[test]
fn endpoint_args_follow_cli_contract() {
    assert_eq!(
        Endpoint::unix("/tmp/pact.sock").args(),
        vec!["--unix".to_owned(), "/tmp/pact.sock".to_owned()]
    );
    assert_eq!(
        Endpoint::tcp("127.0.0.1:8787").args(),
        vec!["--tcp".to_owned(), "127.0.0.1:8787".to_owned()]
    );
    assert_eq!(
        Endpoint::https("127.0.0.1:8787", Some(PathBuf::from("/tmp/ca.pem"))).args(),
        vec![
            "--tcp".to_owned(),
            "127.0.0.1:8787".to_owned(),
            "--https".to_owned(),
            "--tls-ca".to_owned(),
            "/tmp/ca.pem".to_owned(),
        ]
    );
}

#[test]
fn client_error_maps_to_runtime_error() {
    let api: RuntimeError = pact_http::client::ClientError::Api(ErrorDetail {
        code: "IdempotencyConflict".to_owned(),
        detail: serde_json::Value::Null,
        message: "同键异 token".to_owned(),
    })
    .into();
    assert!(matches!(api, RuntimeError::Api { .. }));
    assert!(api.to_string().contains("IdempotencyConflict"));

    let io: RuntimeError = std::io::Error::new(std::io::ErrorKind::NotFound, "no file").into();
    assert!(matches!(io, RuntimeError::Io(_)));
    assert!(io.to_string().contains("no file"));
}

// ============================================================
// T1–T3 集成场景（单 tokio runtime 内顺序执行）
// ============================================================

#[tokio::test(flavor = "multi_thread")]
async fn runtime_scenarios() {
    software_lifecycle().await;
    react_loop().await;
    side_effect_saga().await;
    instance_binding().await;
}

/// T1：REST 编排软件工程生命周期至 Rule-T（论文 Part II §1.5 / Part III Ch.2）。
async fn software_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let sock = spawn_daemon(dir.path()).await;
    let endpoint = Endpoint::unix(&sock);

    let mut orch = Orchestrator::new("swe").with_agent(
        AgentSpec::rest("external_stakeholder", endpoint.clone()).with_bootstrap(Bootstrap {
            declaration: LIFECYCLE.to_owned(),
            seed_type: "requirement".to_owned(),
            thing_ref: Some("stakeholder-need-001".to_owned()),
        }),
    );
    for p in [
        "architect",
        "developer",
        "tester",
        "operator",
        "stakeholder",
    ] {
        orch = orch.with_agent(AgentSpec::rest(p, endpoint.clone()));
    }
    let reports = orch.run().await.expect("编排应全部自驱至终止");
    assert_eq!(reports.len(), 6);

    for r in &reports {
        assert!(r.saw_terminated, "参与者未自行观察到终止：{r:?}");
    }
    assert!(
        reports.iter().any(|r| r.notified_terminated),
        "无参与者收到 Terminated 推送：{reports:?}"
    );

    let total = |f: fn(&AgentReport) -> usize| -> usize { reports.iter().map(f).sum() };
    assert_eq!(total(|r| r.productions), 4, "{reports:?}");
    assert_eq!(total(|r| r.consumptions), 5, "{reports:?}");
    assert_eq!(total(|r| r.reads), 4, "{reports:?}");
    assert!(
        reports.iter().all(|r| r.max_counter <= 5),
        "Counter 卫生被破坏（应恒 ≤ 类型数 5）：{reports:?}"
    );

    let seeder = reports
        .iter()
        .find(|r| r.participant == "external_stakeholder")
        .unwrap();
    assert_eq!((seeder.productions, seeder.consumptions), (0, 0));
    let by_name: BTreeMap<&str, &AgentReport> = reports
        .iter()
        .map(|r| (r.participant.as_str(), r))
        .collect();
    assert_eq!(
        (
            by_name["architect"].productions,
            by_name["architect"].consumptions
        ),
        (1, 1)
    );
    assert_eq!(
        (
            by_name["stakeholder"].productions,
            by_name["stakeholder"].consumptions
        ),
        (0, 1)
    );

    // —— 只读核验 ——
    let c = Client::unix(&sock);
    assert_eq!(
        c.status("swe", None).await.unwrap(),
        StatusView {
            counter_size: 0,
            archive_size: 5,
            terminated: true
        }
    );
    let ar = c.archive("swe", None, None).await.unwrap();
    let role: BTreeMap<&str, (&str, &str)> = ar
        .entries
        .iter()
        .map(|e| {
            (
                e.token.ty.0.as_str(),
                (e.token.produced_by.0.as_str(), e.exhausted_by.0.as_str()),
            )
        })
        .collect();
    assert_eq!(role["requirement"], ("external_stakeholder", "architect"));
    assert_eq!(role["design"], ("architect", "developer"));
    assert_eq!(role["implementation"], ("developer", "tester"));
    assert_eq!(role["test_report"], ("tester", "operator"));
    assert_eq!(role["deployment"], ("operator", "stakeholder"));

    let id_of: BTreeMap<&str, &str> = ar
        .entries
        .iter()
        .map(|e| (e.token.ty.0.as_str(), e.token.id.0.as_str()))
        .collect();
    for (ty, pred) in [
        ("design", "requirement"),
        ("implementation", "design"),
        ("test_report", "implementation"),
        ("deployment", "test_report"),
    ] {
        let e = ar.entries.iter().find(|e| e.token.ty.0 == ty).unwrap();
        assert_eq!(
            e.token
                .depends_on
                .iter()
                .map(|t| t.0.as_str())
                .collect::<Vec<_>>(),
            vec![id_of[pred]],
            "{ty} 的前驱应为 {pred}"
        );
    }
}

/// T2：REST 编排 ReAct 循环（论文 Part III Ch.1），预算 7 模拟 agent 自停判断。
async fn react_loop() {
    let dir = tempfile::tempdir().unwrap();
    let sock = spawn_daemon(dir.path()).await;
    let endpoint = Endpoint::unix(&sock);

    let orch = Orchestrator::new("react")
        .with_agent(
            AgentSpec::rest("user", endpoint.clone()).with_bootstrap(Bootstrap {
                declaration: REACT.to_owned(),
                seed_type: "observation".to_owned(),
                thing_ref: Some("q:capital-of-france".to_owned()),
            }),
        )
        .with_agent(AgentSpec::rest("agent", endpoint.clone()).with_produce_budget(7));
    let reports = orch.run().await.expect("编排应全部自驱至终止");

    for r in &reports {
        assert!(r.saw_terminated, "参与者未自行观察到终止：{r:?}");
    }
    assert!(reports.iter().any(|r| r.notified_terminated), "{reports:?}");

    let (user, agent) = (&reports[0], &reports[1]);
    assert_eq!(
        (user.productions, user.consumptions, user.reads),
        (0, 0, 0),
        "{user:?}"
    );
    assert_eq!(
        (agent.productions, agent.consumptions, agent.reads),
        (7, 8, 7),
        "{agent:?}"
    );
    assert!(
        reports.iter().all(|r| r.max_counter <= 2),
        "Counter 卫生被破坏（单 agent 串行应恒 ≤ 2）：{reports:?}"
    );

    let c = Client::unix(&sock);
    assert_eq!(
        c.status("react", None).await.unwrap(),
        StatusView {
            counter_size: 0,
            archive_size: 8,
            terminated: true
        }
    );
    let ar = c.archive("react", None, None).await.unwrap();
    let types: Vec<&str> = ar.entries.iter().map(|e| e.token.ty.0.as_str()).collect();
    assert_eq!(
        types,
        [
            "observation",
            "thought",
            "action",
            "tool_result",
            "observation",
            "thought",
            "action",
            "tool_result",
        ]
    );
    assert!(ar.entries[0].token.depends_on.is_empty());
    assert!(ar.entries[1..]
        .iter()
        .all(|e| e.token.depends_on.len() == 1));
    assert!(ar.entries[1..]
        .iter()
        .all(|e| e.token.produced_by.0 == "agent"));
}

/// T3：外部副作用执行器——订阅 `Consumed`、落地外部动作、失败追加补偿事实（saga）。
async fn side_effect_saga() {
    let dir = tempfile::tempdir().unwrap();
    let sock = spawn_daemon(dir.path()).await;
    let endpoint = Endpoint::unix(&sock);

    let orch = Orchestrator::new("mat")
        .with_agent(
            AgentSpec::rest("seeder", endpoint.clone()).with_bootstrap(Bootstrap {
                declaration: MATERIALIZATION.to_owned(),
                seed_type: "task".to_owned(),
                thing_ref: Some("task-1".to_owned()),
            }),
        )
        .with_agent(AgentSpec::rest("worker", endpoint.clone()).with_produce_budget(1))
        .with_agent(AgentSpec::rest("sink", endpoint.clone()));

    let ex_cfg = ExecutorConfig::new("materializer")
        .watching(["task", "artifact"])
        .with_compensation(Compensation {
            participant: "materializer".to_owned(),
            ty: "task_failure".to_owned(),
            finalize: true,
        });
    let client = RestClient::from_endpoint(&endpoint, "mat", None).expect("unix 端点构造应成功");
    let mut executor = SideEffectExecutor::new(client, ex_cfg);
    let stop = StopSignal::new();
    let stop_for_exec = stop.clone();
    let exec_fut = async move {
        let handler = FailingHandler {
            fail_on: "task".to_owned(),
        };
        let r = executor.run(&handler, &stop_for_exec).await;
        stop.stop();
        r
    };

    let (agents_res, exec_res) = tokio::join!(orch.run(), exec_fut);
    let reports = agents_res.expect("编排应全部自驱至终止");
    let erep = exec_res.expect("执行器应正常退出");

    for r in &reports {
        assert!(r.saw_terminated, "参与者未自行观察到终止：{r:?}");
    }
    assert_eq!(erep.applied, 1, "artifact 外部动作成功：{erep:?}");
    assert_eq!(erep.failed, 1, "task 外部动作失败：{erep:?}");
    assert_eq!(erep.compensated, 1, "应追加 1 条补偿事实：{erep:?}");
    assert!(erep.saw_terminated, "执行器应观察到终止：{erep:?}");

    // —— 只读核验：补偿事实已归档，且与被失败的消费关联 ——
    let c = Client::unix(&sock);
    let ar = c.archive("mat", None, None).await.unwrap();
    let task_id = ar
        .entries
        .iter()
        .find(|e| e.token.ty.0 == "task")
        .expect("task 应已归档")
        .token
        .id
        .0
        .clone();
    let failures: Vec<&ArchiveEntry> = ar
        .entries
        .iter()
        .filter(|e| e.token.ty.0 == "task_failure")
        .collect();
    let observed: Vec<String> = ar.entries.iter().map(|e| e.token.ty.0.clone()).collect();
    assert_eq!(failures.len(), 1, "补偿事实应恰一条：{observed:?}");
    assert_eq!(failures[0].token.produced_by.0, "materializer");
    assert!(failures[0].token.depends_on.is_empty(), "Rule-S 无前驱");
    assert_eq!(
        failures[0].token.thing_ref.0.as_deref(),
        Some(task_id.as_str()),
        "补偿事实应以 thing_ref 关联被失败的 token"
    );
    assert_eq!(
        c.status("mat", None).await.unwrap(),
        StatusView {
            counter_size: 0,
            archive_size: 3,
            terminated: true
        }
    );
}

/// T4：域双键（D27）——同一域声明并发承载两个实例，运行态互不干扰。
async fn instance_binding() {
    let dir = tempfile::tempdir().unwrap();
    let sock = spawn_daemon(dir.path()).await;
    let endpoint = Endpoint::unix(&sock);

    // 构造绑定到「swe/<instance>」的整套编排（引导者自动载入声明）
    let build = |instance: Option<&str>| {
        let mut orch = Orchestrator::new("swe");
        if let Some(i) = instance {
            orch = orch.with_instance(i.to_owned());
        }
        orch = orch.with_agent(
            AgentSpec::rest("external_stakeholder", endpoint.clone()).with_bootstrap(Bootstrap {
                declaration: LIFECYCLE.to_owned(),
                seed_type: "requirement".to_owned(),
                thing_ref: Some("need".to_owned()),
            }),
        );
        for p in [
            "architect",
            "developer",
            "tester",
            "operator",
            "stakeholder",
        ] {
            orch = orch.with_agent(AgentSpec::rest(p, endpoint.clone()));
        }
        orch
    };

    // default 与 alt 两实例并发独立自驱（各自引导并跑完整生命周期）
    let (r_def, r_alt) = tokio::join!(build(None).run(), build(Some("alt")).run());
    let def = r_def.expect("default 实例应终止");
    let alt = r_alt.expect("alt 实例应终止");
    assert_eq!((def.len(), alt.len()), (6, 6));
    for r in def.iter().chain(alt.iter()) {
        assert!(r.saw_terminated, "参与者未观察到终止：{r:?}");
    }

    // 只读核验：两实例各自独立归档（各 5 条），未串扰
    let c = Client::unix(&sock);
    assert_eq!(c.archive("swe", None, None).await.unwrap().size, 5);
    assert_eq!(c.archive("swe", Some("alt"), None).await.unwrap().size, 5);
    let st_def = c.status("swe", None).await.unwrap();
    let st_alt = c.status("swe", Some("alt")).await.unwrap();
    assert!(st_def.terminated && st_alt.terminated);

    // 清单：一个定义键、两个实例
    let list = c.list_domains().await.unwrap();
    assert_eq!(list.domains.len(), 1);
    assert_eq!(list.domains[0].name, "swe");
    assert_eq!(list.domains[0].instances, vec!["alt", "default"]);

    // 存储层：两实例日志分文件，各自完整记录生命周期（含 Terminated）
    let data = dir.path().join("data").join("swe");
    for f in ["default.events.jsonl", "alt.events.jsonl"] {
        let text = std::fs::read_to_string(data.join(f)).unwrap();
        assert!(
            text.contains("\"kind\":\"DomainLoaded\""),
            "{f} 缺 DomainLoaded"
        );
        assert!(
            text.contains("\"kind\":\"Terminated\""),
            "{f} 缺 Terminated"
        );
    }
}
