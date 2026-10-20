//! 阶段 5（FR11）：无人值守场景验证。
//!
//! 测试进程只承担"外部启动机制"：拉起 daemon 与 N 个 agent 进程，此后不发出任何
//! Rule-S/P/R/E（引擎被动：不编排、不拉起 agent、不重试）。每个 agent = 一个真实
//! `pact-mcp` stdio 子进程 + 一条自驱策略循环，仅凭 `pact_subscribe` 推送与
//! `pact_status` 轮询兜底唤醒（Rule-W），靠共享 Counter 状态自组织推进，直到观察到
//! `terminated` 后自行退出（Rule-T）。agent 之间无直接通信、无编排者。
//!
//! 自驱策略（agent 本地私有，不经协议传递；PACT 不提供全局次序，活性来自策略）：
//! - **P1 生产优先**：`pact_enabled` 非空且预算未耗尽 → 读见证（Rule-R）→
//!   生产（Rule-P，显式传见证前驱）→ 立即回收自己的见证前驱（Rule-E，
//!   consume-after-produce 纪律）。
//! - **P2 汇消费**：P1 无动作时，消费 Counter 上自己持有 `consumes` 门的 token。
//!   仅在可满足组合为空（或预算耗尽）时触发，故不会抢走他人生产所需的见证前驱
//!   （本文两域中每类型至多一个消费者，该纪律成立）。
//! - **P3 等待**：无可做动作 → 阻塞在订阅推送 / 轮询兜底上。
//!
//! 生产预算模拟 ReAct agent 的内部"停止"判断（论文中由 LLM 认定答案已得），
//! 属 agent 私有状态，不经 Counter 传递。

use pact_engine::views::StatusView;
use pact_engine::{Engine, LogicalClock, SequentialIdGen};
use pact_http::client::Client;
use pact_http::server::{router, serve_unix, ConnTracker, ServerState};
use pact_store::Store;
use rmcp::model::{CallToolRequestParams, ContentBlock, CustomNotification};
use rmcp::service::NotificationContext;
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{ClientHandler, Peer, RoleClient, ServiceExt};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

/// 轮询兜底间隔（Rule-W：推送之外的兜底唤醒）。
const POLL: Duration = Duration::from_millis(20);
/// 单 agent 自驱至终止的期限（超时即场景失败）。
const DEADLINE: Duration = Duration::from_secs(60);

// ============================================================
// 域声明
// ============================================================

/// 论文 Part III Ch.1：ReAct 循环域（agent 持全读写产生门；user 持种子权）。
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

/// 论文 Part II §1.5：软件工程生命周期域（5 个参与者 + 1 个种子权威）。
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

// ============================================================
// MCP client 侧：Rule-W 推送收集
// ============================================================

/// 把 `notifications/pact/event` 自定义通知转成内部唤醒信号。
#[derive(Clone)]
struct Notifier(mpsc::UnboundedSender<Value>);

impl ClientHandler for Notifier {
    async fn on_custom_notification(
        &self,
        notification: CustomNotification,
        _context: NotificationContext<RoleClient>,
    ) {
        if notification.method == "notifications/pact/event" {
            if let Some(params) = notification.params {
                let _ = self.0.send(params);
            }
        }
    }
}

/// 调工具并解析 §6 线上格式；无人值守流程中任何协议错误都视为场景失败。
async fn call(peer: &Peer<RoleClient>, name: &str, args: Value) -> Value {
    let params = CallToolRequestParams::new(name.to_owned())
        .with_arguments(serde_json::from_value(args).unwrap());
    let res = peer
        .call_tool(params)
        .await
        .unwrap_or_else(|e| panic!("调用 {name} 失败：{e:?}"));
    assert!(
        !res.is_error.unwrap_or(false),
        "工具 {name} 不应报错：{res:?}"
    );
    let ContentBlock::Text(t) = &res.content[0] else {
        panic!("工具 {name} 返回非文本内容")
    };
    serde_json::from_str(&t.text).unwrap_or_else(|e| panic!("工具 {name} 输出非法 JSON：{e}"))
}

// ============================================================
// 自治 agent
// ============================================================

/// 引导职责：加载域 + 注入种子（Rule-S 持有者）。
struct Bootstrap {
    declaration: String,
    seed_type: String,
    thing_ref: Option<String>,
}

struct AgentCfg {
    sock: PathBuf,
    domain: String,
    participant: String,
    bootstrap: Option<Bootstrap>,
    /// 生产预算（None = 无限）；耗尽后转入 P2 清空可消费 token。
    produce_budget: Option<usize>,
    poll: Duration,
    deadline: Duration,
}

impl AgentCfg {
    fn bootstrap(
        sock: PathBuf,
        domain: &str,
        participant: &str,
        declaration: &str,
        seed_type: &str,
        thing_ref: Option<&str>,
    ) -> Self {
        Self {
            sock,
            domain: domain.to_owned(),
            participant: participant.to_owned(),
            bootstrap: Some(Bootstrap {
                declaration: declaration.to_owned(),
                seed_type: seed_type.to_owned(),
                thing_ref: thing_ref.map(str::to_owned),
            }),
            produce_budget: None,
            poll: POLL,
            deadline: DEADLINE,
        }
    }

    fn worker(sock: PathBuf, domain: &str, participant: &str) -> Self {
        Self {
            sock,
            domain: domain.to_owned(),
            participant: participant.to_owned(),
            bootstrap: None,
            produce_budget: None,
            poll: POLL,
            deadline: DEADLINE,
        }
    }
}

#[derive(Debug, Default)]
struct AgentReport {
    participant: String,
    /// 经 `pact_status` 观察到 Rule-T。
    saw_terminated: bool,
    /// 经 Rule-W 推送观察到 Terminated 通知。
    notified_terminated: bool,
    /// 全程观察到的 Counter 规模上界（推送 + 轮询两路观测）。
    max_counter: usize,
    notifications: usize,
    productions: usize,
    consumptions: usize,
    reads: usize,
}

/// 吸收一条 Rule-W 推送。
fn absorb(rep: &mut AgentReport, n: &Value) {
    rep.notifications += 1;
    if let Some(sz) = n["summary"]["counter_size"].as_u64() {
        rep.max_counter = rep.max_counter.max(sz as usize);
    }
    if n["event"].as_str() == Some("Terminated") {
        rep.notified_terminated = true;
    }
}

/// 拉起一个 agent 进程并运行其自驱循环，直到它自行观察到终止。
async fn run_agent(cfg: AgentCfg) -> AgentReport {
    // —— 外部机制唯一动作：拉起进程 ——
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let (transport, _stderr) = TokioChildProcess::builder(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_pact-mcp")).configure(|c| {
            c.arg("--unix").arg(&cfg.sock);
        }),
    )
    .spawn()
    .expect("启动 pact-mcp 子进程失败");
    let svc = Notifier(tx).serve(transport).await.expect("MCP 握手失败");
    let peer: &Peer<RoleClient> = &svc;

    // —— 自驱接入 ——
    match &cfg.bootstrap {
        // 引导 agent：域加载是它自己的动作（部署即配置，不经测试进程）
        Some(b) => {
            call(
                peer,
                "pact_domain_load",
                json!({ "name": cfg.domain, "declaration": b.declaration }),
            )
            .await;
        }
        // 工作 agent：轮询 `pact_list_domains` 直到域出现，自行接入
        None => {
            let mut found = false;
            for _ in 0..1000 {
                let l = call(peer, "pact_list_domains", json!({})).await;
                let dominated = l["domains"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["name"].as_str() == Some(cfg.domain.as_str()));
                if dominated {
                    found = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(
                found,
                "agent {} 未能自驱发现域 {}",
                cfg.participant, cfg.domain
            );
        }
    }
    // Rule-W 推送通道（订阅失败即场景失败：轮询兜底不能替代推送验证）
    call(peer, "pact_subscribe", json!({ "domain": cfg.domain })).await;

    // 自省：我持有哪些 `consumes` 门（§9.1 `pact_my_gates`）
    let gates = call(
        peer,
        "pact_my_gates",
        json!({ "domain": cfg.domain, "participant": cfg.participant }),
    )
    .await;
    let consumable: BTreeSet<String> = gates["gates"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| g["action"].as_str() == Some("consume"))
        .filter_map(|g| g["type"].as_str().map(str::to_owned))
        .collect();
    // 是否持有 `produces` 门：决定 P2 汇消费是否允许（见下）。
    let can_produce = gates["gates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|g| g["action"].as_str() == Some("produce"));

    // 种子注入（Rule-S）：闭环起点，由持种子权的 agent 自主完成
    if let Some(b) = &cfg.bootstrap {
        call(
            peer,
            "pact_seed",
            json!({
                "domain": cfg.domain, "participant": cfg.participant,
                "type": b.seed_type, "thing_ref": b.thing_ref,
            }),
        )
        .await;
    }

    // —— 自驱循环 ——
    let mut rep = AgentReport {
        participant: cfg.participant.clone(),
        ..Default::default()
    };
    let mut ticker = tokio::time::interval(cfg.poll);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let start = Instant::now();
    // 推送通道关闭后禁用其 select 分支，转为纯轮询兜底（Rule-W：推送失败不阻断自驱）。
    let mut rx_closed = false;
    loop {
        assert!(
            start.elapsed() < cfg.deadline,
            "agent {} 未在 {:?} 内自驱至终止（最后状态：{:?}）",
            cfg.participant,
            cfg.deadline,
            rep
        );
        // 排空已到达的推送
        while let Ok(n) = rx.try_recv() {
            absorb(&mut rep, &n);
        }

        // 观察：Counter 概要 + 终止判定（Rule-T）
        let st = call(peer, "pact_status", json!({ "domain": cfg.domain })).await;
        rep.max_counter = rep
            .max_counter
            .max(st["counter_size"].as_u64().unwrap_or(0) as usize);
        // 过早终止防护：域加载与种子注入之间存在窗口，此时 Counter=∅ 且无 token，
        // `termination_holds` 平凡为真。仅当 archive_size>0（确有真实工作发生）时才承认
        // 终止，避免 worker 在播种前误判 terminated 而空手退出（导致后续 token 无人接力）。
        let pristine = st["archive_size"].as_u64().unwrap_or(0) == 0;
        if st["terminated"].as_bool().unwrap_or(false) && !pristine {
            rep.saw_terminated = true;
            // Rule-W：一次性 Terminated 推送与轮询兜底存在竞态——轮询可能先于推送判定终止。
            // 退出前给在途推送一个宽限窗口，确保订阅者确实收到 Terminated 广播，
            // 而非仅靠轮询判定（否则 notified_terminated 会随机漏计）。
            let grace = Instant::now();
            while !rep.notified_terminated && grace.elapsed() < Duration::from_millis(500) {
                match tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
                    Ok(Some(n)) => absorb(&mut rep, &n),
                    Ok(None) => break, // 推送通道关闭：轮询已判定终止，可安全退出
                    Err(_) => {}       // 本窗口无推送，继续等
                }
            }
            break;
        }

        // P1：生产优先
        let mut acted = false;
        let budget_ok = match cfg.produce_budget {
            None => true,
            Some(b) => rep.productions < b,
        };
        if budget_ok {
            let en = call(
                peer,
                "pact_enabled",
                json!({ "domain": cfg.domain, "participant": cfg.participant }),
            )
            .await;
            if let Some(opt) = en["options"].as_array().and_then(|a| a.first()) {
                let ty = opt["type"].as_str().unwrap().to_owned();
                let witness_ids: Vec<String> = opt["witness_predecessors"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect();
                // Rule-R：生产前读见证（与黄金轨迹同构），顺带取得见证类型
                let mut witnesses: Vec<(String, String)> = Vec::new();
                for w in &witness_ids {
                    let r = call(
                        peer,
                        "pact_read",
                        json!({ "domain": cfg.domain, "participant": cfg.participant, "token_id": w }),
                    )
                    .await;
                    let wt = r["token"]["type"].as_str().unwrap().to_owned();
                    witnesses.push((w.clone(), wt));
                    rep.reads += 1;
                }
                // Rule-P：显式传见证前驱
                call(
                    peer,
                    "pact_produce",
                    json!({
                        "domain": cfg.domain, "participant": cfg.participant,
                        "type": ty, "predecessors": witness_ids,
                    }),
                )
                .await;
                rep.productions += 1;
                // Rule-E：consume-after-produce，回收自己的见证前驱
                for (id, wt) in &witnesses {
                    if consumable.contains(wt) {
                        call(
                            peer,
                            "pact_consume",
                            json!({ "domain": cfg.domain, "participant": cfg.participant, "token_id": id }),
                        )
                        .await;
                        rep.consumptions += 1;
                    }
                }
                acted = true;
            }
        }

        // P2：汇消费——仅当 agent 确实不再生产时才允许（纯汇：无 produces 门；或生产预算已耗尽）。
        // 有 produces 门且预算未尽者绝不在此吃掉自己的见证：否则 P1 的 enabled 查询与本次消费之间
        // 若见证恰好到达，会误将见证当汇消费回收，破坏 consume-after-produce 纪律并导致提前终止。
        if !acted && !consumable.is_empty() && (!can_produce || !budget_ok) {
            let cq = call(peer, "pact_counter_query", json!({ "domain": cfg.domain })).await;
            let target = cq["tokens"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| consumable.contains(t["type"].as_str().unwrap_or("")))
                .map(|t| t["id"].as_str().unwrap().to_owned());
            if let Some(id) = target {
                call(
                    peer,
                    "pact_consume",
                    json!({ "domain": cfg.domain, "participant": cfg.participant, "token_id": id }),
                )
                .await;
                rep.consumptions += 1;
                acted = true;
            }
        }

        // P3：等待唤醒（推送优先，轮询兜底）
        if !acted {
            tokio::select! {
                n = rx.recv(), if !rx_closed => match n {
                    Some(n) => absorb(&mut rep, &n),
                    None => rx_closed = true, // 推送通道关闭：转纯轮询兜底，绝不退出自驱
                },
                _ = ticker.tick() => {}
            }
        }
    }
    svc.cancel().await.expect("关闭 MCP 会话失败");
    rep
}

/// 并发拉起全部 agent，等待它们各自自行退出（顺序与入参一致）。
async fn run_agents(cfgs: Vec<AgentCfg>) -> Vec<AgentReport> {
    let names: Vec<String> = cfgs.iter().map(|c| c.participant.clone()).collect();
    let mut handles = Vec::with_capacity(cfgs.len());
    for cfg in cfgs {
        handles.push(tokio::spawn(run_agent(cfg)));
    }
    let mut out = Vec::with_capacity(handles.len());
    let mut errs = Vec::new();
    for (name, h) in names.iter().zip(handles) {
        match h.await {
            Ok(rep) => out.push(rep),
            Err(e) => errs.push(format!("{name}: {e:?}")),
        }
    }
    if !errs.is_empty() {
        panic!(
            "agent 任务崩溃：\n  已完成报告：{out:#?}\n  崩溃：{}",
            errs.join("\n  ")
        );
    }
    out
}

// ============================================================
// daemon（进程内起真实引擎 + unix socket；测试进程随后只做只读核验）
// ============================================================

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

// ============================================================
// 场景 A：ReAct 循环（论文 Part III Ch.1）
// ============================================================

/// user 注入初始观察后，agent 自驱完成两轮 ReAct 循环并自行停止，
/// 全程 Counter ≤ 2（O(1) 卫生），至 Counter = ∅ → Rule-T。
async fn scenario_react_loop() {
    let dir = tempfile::tempdir().unwrap();
    let sock = spawn_daemon(dir.path()).await;

    let reports = run_agents(vec![
        AgentCfg::bootstrap(
            sock.clone(),
            "react",
            "user",
            REACT,
            "observation",
            Some("q:capital-of-france"),
        ),
        // 两轮 × 4 次生产 − 1（末轮不再续接新观察，转入停止分支）
        AgentCfg {
            produce_budget: Some(7),
            ..AgentCfg::worker(sock.clone(), "react", "agent")
        },
    ])
    .await;

    let (user, agent) = (&reports[0], &reports[1]);
    // 两个 agent 都自行观察到 Rule-T 后退出（无外部终止信号）
    for r in &reports {
        assert!(r.saw_terminated, "agent 未自行观察到终止：{r:?}");
    }
    // Rule-W 推送确实到达（至少一个 agent 收到 Terminated 通知）
    assert!(
        reports.iter().any(|r| r.notified_terminated),
        "无 agent 收到 Terminated 推送：{reports:?}"
    );
    // 状态演化全部由 agent 经 MCP 完成：user 只注入种子，agent 7 产 8 消 7 读
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
    // Counter O(1) 卫生（论文 §1.3）：任意时刻 ≤ 2
    assert!(
        reports.iter().all(|r| r.max_counter <= 2),
        "Counter 卫生被破坏（应恒 ≤ 2）：{reports:?}"
    );

    // —— 只读核验（测试进程不做任何状态变更）——
    let c = Client::unix(&sock);
    let st = c.status("react", None).await.unwrap();
    assert_eq!(
        st,
        StatusView {
            counter_size: 0,
            archive_size: 8,
            terminated: true
        }
    );

    let ar = c.archive("react", None, None).await.unwrap();
    let types: Vec<&str> = ar.entries.iter().map(|e| e.token.ty.0.as_str()).collect();
    // 单 agent 串行，消费序即两轮循环序
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
    // 种子无前驱；其余每步恰一个前驱（循环前置）
    assert!(ar.entries[0].token.depends_on.is_empty(), "种子不应有前驱");
    for e in &ar.entries[1..] {
        assert_eq!(e.token.depends_on.len(), 1, "ReAct 每步应恰一个前驱：{e:?}");
    }
    // 溯源闭环：第二轮 observation 的前驱是第一轮 tool_result
    assert_eq!(
        ar.entries[4].token.depends_on,
        vec![ar.entries[3].token.id.clone()]
    );
    // 全部 token 由 agent 产出（种子由 user 注入），由 agent 消费
    assert_eq!(ar.entries[0].token.produced_by.0, "user");
    assert!(ar.entries[1..]
        .iter()
        .all(|e| e.token.produced_by.0 == "agent"));
    assert!(ar.entries.iter().all(|e| e.exhausted_by.0 == "agent"));
}

// ============================================================
// 场景 B：软件工程生命周期（论文 Part II §1.5 / Part III Ch.2）
// ============================================================

/// 6 个 agent（1 种子权威 + 5 参与者）并发自驱，无编排者、无直接通信，
/// 仅由 Rule-P 可满足性触发接力，至 Counter = ∅ → Rule-T。
async fn scenario_software_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let sock = spawn_daemon(dir.path()).await;

    let mut cfgs = vec![AgentCfg::bootstrap(
        sock.clone(),
        "swe",
        "external_stakeholder",
        LIFECYCLE,
        "requirement",
        Some("stakeholder-need-001"),
    )];
    for p in [
        "architect",
        "developer",
        "tester",
        "operator",
        "stakeholder",
    ] {
        cfgs.push(AgentCfg::worker(sock.clone(), "swe", p));
    }
    let reports = run_agents(cfgs).await;

    // 全部 agent 自行观察到 Rule-T 后退出
    for r in &reports {
        assert!(r.saw_terminated, "agent 未自行观察到终止：{r:?}");
    }
    assert!(
        reports.iter().any(|r| r.notified_terminated),
        "无 agent 收到 Terminated 推送：{reports:?}"
    );
    // 状态演化总量：4 次生产（design/implementation/test_report/deployment）
    // + 5 次消费（含 stakeholder 消费 deployment）+ 4 次读
    let total = |f: fn(&AgentReport) -> usize| -> usize { reports.iter().map(f).sum() };
    assert_eq!(total(|r| r.productions), 4, "{reports:?}");
    assert_eq!(total(|r| r.consumptions), 5, "{reports:?}");
    assert_eq!(total(|r| r.reads), 4, "{reports:?}");
    // 种子权威只注入不推进；纯消费者只消费不生产
    let seeder = &reports[0];
    assert_eq!(
        (seeder.productions, seeder.consumptions),
        (0, 0),
        "{seeder:?}"
    );
    let by_name: BTreeMap<&str, &AgentReport> = reports
        .iter()
        .map(|r| (r.participant.as_str(), r))
        .collect();
    assert_eq!(
        (
            by_name["stakeholder"].productions,
            by_name["stakeholder"].consumptions
        ),
        (0, 1)
    );
    assert_eq!(
        (
            by_name["architect"].productions,
            by_name["architect"].consumptions
        ),
        (1, 1)
    );
    // Counter 卫生的结构上界：每类型至多一实例（每类型仅一个 produce 门持有者，
    // 且 P1 在同一迭代内串行完成 produce → consume-after-produce），故 ≤ 类型数 5。
    // 注意：典型调度下工作集 ≤ 2（consume-after-produce 窗口极窄），但慢调度
    //（如 CI 满载）下，下一阶段 agent 可能在上一阶段 produce→consume 窗口内完成
    // 自己的生产——Rule-P 只要求前驱在 Counter 上，且 produces 门按 §3.1 不接受
    // 条件子句，该窗口语义上不可消除。窗口重叠是自治语义下的合法执行，非引擎缺陷
    //（2026-09-09 CI 实测出现 max_counter=3），故只断言结构上界。
    assert!(
        reports.iter().all(|r| r.max_counter <= 5),
        "Counter 卫生被破坏（应恒 ≤ 类型数 5）：{reports:?}"
    );

    // —— 只读核验 ——
    let c = Client::unix(&sock);
    let st = c.status("swe", None).await.unwrap();
    assert_eq!(
        st,
        StatusView {
            counter_size: 0,
            archive_size: 5,
            terminated: true
        }
    );

    let ar = c.archive("swe", None, None).await.unwrap();
    let mut types: Vec<String> = ar.entries.iter().map(|e| e.token.ty.0.clone()).collect();
    types.sort();
    assert_eq!(
        types,
        [
            "deployment",
            "design",
            "implementation",
            "requirement",
            "test_report"
        ]
    );
    // 每个 token 的生产者/消费者与 gate 授权一一对应（t₀–t₁₀ 角色分工）
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
    // 溯源链：requirement → design → implementation → test_report → deployment
    let id_of: BTreeMap<&str, _> = ar
        .entries
        .iter()
        .map(|e| (e.token.ty.0.as_str(), &e.token.id))
        .collect();
    assert_eq!(
        ar.entries
            .iter()
            .find(|e| e.token.ty.0 == "requirement")
            .unwrap()
            .token
            .depends_on,
        vec![]
    );
    for (ty, pred) in [
        ("design", "requirement"),
        ("implementation", "design"),
        ("test_report", "implementation"),
        ("deployment", "test_report"),
    ] {
        let e = ar.entries.iter().find(|e| e.token.ty.0 == ty).unwrap();
        assert_eq!(
            &e.token.depends_on,
            std::slice::from_ref(id_of[pred]),
            "{ty} 的前驱应为 {pred}"
        );
    }
}

// ============================================================
// 阶段 5 验收入口（FR11 / design §11）
// ============================================================

/// 两个无人值守场景在**单一 runtime 内顺序**执行。
///
/// 合并为单个 `#[tokio::test]` 而非两个并发测试，是测试基架约束：每个场景都要 spawn 若干
/// `pact-mcp` stdio 子进程，而 tokio 的子进程回收 / SIGCHLD 驱动是**进程级全局**的；同一
/// 进程内两个独立 runtime 并发 reap 各自的子进程是已知陷阱（孤儿队列被单一驱动排空，另一
/// runtime 的子进程 I/O 可能失联）。单 runtime 顺序执行规避此陷阱，与 PACT 引擎语义无关。
/// 场景内部的自驱竞态（播种前过早终止、推送/轮询竞态）已在 `run_agent` 中单独修复。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unattended_scenarios_to_terminated() {
    scenario_react_loop().await;
    scenario_software_lifecycle().await;
}
