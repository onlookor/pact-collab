//! 可复用的参与者自驱循环（P1 生产优先 / P2 汇消费 / P3 等待）。
//!
//! 便携化自 `crates/pact-mcp/tests/unattended_e2e.rs` 的 `run_agent`：剥离进程与传输构造，
//! 仅保留「发现域 → 订阅 → 取门 → （引导）播种 → 决策循环 → 关闭」的语义主干；差异仅在
//! 协议错误以 [`RuntimeError`] 返回而非 panic，传输由注入的 [`ParticipantClient`] 提供。

use crate::client::ParticipantClient;
use crate::error::RuntimeError;
use pact_core::Action;
use pact_engine::notify::{Notification, NotificationKind};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

/// 默认轮询兜底间隔（Rule-W：推送之外的兜底唤醒）。
pub const DEFAULT_POLL: Duration = Duration::from_millis(20);
/// 默认自驱期限（超时即视为未收敛）。
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(60);
/// 终止宽限窗口：等待在途推送到达（避免漏记 Terminated 通知）。
const TERMINATION_GRACE: Duration = Duration::from_millis(500);
/// 等待域出现的轮询次数与间隔。
const DISCOVERY_TRIES: usize = 1000;
const DISCOVERY_INTERVAL: Duration = Duration::from_millis(10);

/// 引导参与者配置：**域加载是它自己的动作**（部署即配置），并注入初始种子。
#[derive(Debug, Clone)]
pub struct Bootstrap {
    /// `.pact` 域声明文本。
    pub declaration: String,
    /// 种子类型（须为源类型且参与者持 seed 权）。
    pub seed_type: String,
    /// 可选外部事物引用。
    pub thing_ref: Option<String>,
}

/// 驱动配置。
#[derive(Debug, Clone)]
pub struct DriverConfig {
    /// 参与者名（域内身份）。
    pub participant: String,
    /// 引导配置；非引导参与者为 `None`（等待域出现即可）。
    pub bootstrap: Option<Bootstrap>,
    /// Rule-P 产出预算；`None` = 不限（域耗尽即止）。
    pub produce_budget: Option<usize>,
    /// 轮询兜底间隔。
    pub poll: Duration,
    /// 自驱期限。
    pub deadline: Duration,
}

impl DriverConfig {
    /// 以参与者名构造（默认轮询/期限）。
    pub fn new(participant: impl Into<String>) -> Self {
        Self {
            participant: participant.into(),
            bootstrap: None,
            produce_budget: None,
            poll: DEFAULT_POLL,
            deadline: DEFAULT_DEADLINE,
        }
    }

    /// 设为引导参与者。
    pub fn with_bootstrap(mut self, bootstrap: Bootstrap) -> Self {
        self.bootstrap = Some(bootstrap);
        self
    }

    /// 设 Rule-P 产出预算。
    pub fn with_produce_budget(mut self, n: usize) -> Self {
        self.produce_budget = Some(n);
        self
    }

    /// 设轮询兜底间隔。
    pub fn with_poll(mut self, poll: Duration) -> Self {
        self.poll = poll;
        self
    }

    /// 设自驱期限。
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }
}

/// 单 agent 自驱结果。
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AgentReport {
    /// 参与者名。
    pub participant: String,
    /// 自身查询观察到 Rule-T 终止。
    pub saw_terminated: bool,
    /// 收到 `Terminated` 推送通知（Rule-W 唤醒确实到达）。
    pub notified_terminated: bool,
    /// 观察到的 Counter 峰值。
    pub max_counter: usize,
    /// 收到的推送通知条数。
    pub notifications: usize,
    /// Rule-P 产出次数。
    pub productions: usize,
    /// Rule-E 消费次数。
    pub consumptions: usize,
    /// Rule-R 读取次数。
    pub reads: usize,
}

/// 吸收一条推送通知。
pub fn absorb(rep: &mut AgentReport, n: &Notification) {
    rep.notifications += 1;
    rep.max_counter = rep.max_counter.max(n.summary.counter_size);
    if n.event == NotificationKind::Terminated {
        rep.notified_terminated = true;
    }
}

/// 驱动单个参与者自驱至终止（或期限耗尽）。
pub async fn drive<C: ParticipantClient>(
    mut client: C,
    cfg: &DriverConfig,
) -> Result<AgentReport, RuntimeError> {
    let domain = client.domain().to_owned();

    // —— 引导：加载域 或 等待域出现 ——
    match &cfg.bootstrap {
        Some(b) => {
            client.load_domain(&b.declaration).await?;
        }
        None => {
            let mut found = false;
            for _ in 0..DISCOVERY_TRIES {
                if client.list_domains().await?.iter().any(|d| d == &domain) {
                    found = true;
                    break;
                }
                tokio::time::sleep(DISCOVERY_INTERVAL).await;
            }
            if !found {
                return Err(RuntimeError::Timeout(format!(
                    "参与者 {} 未发现域 {domain}",
                    cfg.participant
                )));
            }
        }
    }

    let mut wake = client.subscribe().await?;

    // —— 参与者能力：可消费类型集 + 是否可产出 ——
    let gates = client.my_gates(&cfg.participant).await?;
    let consumable: BTreeSet<String> = gates
        .iter()
        .filter(|g| g.action == Action::Consume)
        .map(|g| g.ty.0.clone())
        .collect();
    let can_produce = gates.iter().any(|g| g.action == Action::Produce);

    // —— 种子注入（仅引导参与者）——
    if let Some(b) = &cfg.bootstrap {
        client
            .seed(&cfg.participant, &b.seed_type, b.thing_ref.as_deref())
            .await?;
    }

    let mut rep = AgentReport {
        participant: cfg.participant.clone(),
        ..Default::default()
    };
    let mut ticker = tokio::time::interval(cfg.poll);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let start = Instant::now();
    let mut wake_closed = false;

    loop {
        if start.elapsed() >= cfg.deadline {
            return Err(RuntimeError::Timeout(format!(
                "参与者 {} 未在 {:?} 内自驱至终止（最后报告：{rep:?}）",
                cfg.participant, cfg.deadline
            )));
        }

        // 排空已就绪的在途推送。
        while let Some(n) = wake.try_recv() {
            absorb(&mut rep, &n);
        }

        let st = client.status().await?;
        rep.max_counter = rep.max_counter.max(st.counter_size);
        // pristine 守卫：域刚加载、尚无任何消费时 Rule-T 判定成立，但此时并非真正的终止。
        let pristine = st.archive_size == 0;
        if st.terminated && !pristine {
            rep.saw_terminated = true;
            let grace = Instant::now();
            while !rep.notified_terminated && grace.elapsed() < TERMINATION_GRACE {
                match tokio::time::timeout(Duration::from_millis(50), wake.recv()).await {
                    Ok(Some(n)) => absorb(&mut rep, &n),
                    Ok(None) => break,
                    Err(_) => {}
                }
            }
            break;
        }

        let budget_ok = match cfg.produce_budget {
            None => true,
            Some(b) => rep.productions < b,
        };

        // —— P1：生产优先（读见证 → 产出 → 消费可消费见证）——
        let mut acted = false;
        if budget_ok {
            let options = client.enabled(&cfg.participant).await?;
            if let Some(opt) = options.first() {
                let ty = opt.ty.0.clone();
                let witness_ids: Vec<String> = opt
                    .witness_predecessors
                    .iter()
                    .map(|t| t.0.clone())
                    .collect();
                let mut witnesses: Vec<(String, String)> = Vec::with_capacity(witness_ids.len());
                for w in &witness_ids {
                    let t = client.read(&cfg.participant, w).await?;
                    witnesses.push((w.clone(), t.ty.0.clone()));
                    rep.reads += 1;
                }
                client.produce(&cfg.participant, &ty, &witness_ids).await?;
                rep.productions += 1;
                for (id, wty) in &witnesses {
                    if consumable.contains(wty) {
                        client.consume(&cfg.participant, id, None).await?;
                        rep.consumptions += 1;
                    }
                }
                acted = true;
            }
        }

        // —— P2：汇消费（无可产出或预算已尽时，消费 Counter 上最靠前的可消费 token）——
        if !acted && !consumable.is_empty() && (!can_produce || !budget_ok) {
            let counter = client.counter().await?;
            if let Some(id) = counter
                .tokens
                .iter()
                .find(|t| consumable.contains(&t.ty.0))
                .map(|t| t.id.0.clone())
            {
                client.consume(&cfg.participant, &id, None).await?;
                rep.consumptions += 1;
                acted = true;
            }
        }

        // —— P3：等待（推送优先，轮询兜底）——
        if !acted {
            tokio::select! {
                n = wake.recv(), if !wake_closed => match n {
                    Some(n) => absorb(&mut rep, &n),
                    None => wake_closed = true,
                },
                _ = ticker.tick() => {}
            }
        }
    }

    client.close().await?;
    Ok(rep)
}
