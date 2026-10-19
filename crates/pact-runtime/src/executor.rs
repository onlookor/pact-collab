//! 外部副作用执行器（outbox/saga 消费端）。
//!
//! 边界（FR15 / D23）：引擎不执行外部副作用，**消费成功与外部动作成功是两个独立承诺**。
//! 执行器订阅 `Consumed` 通知、落地外部动作；失败时**不表现为消费回滚**，而是以 Rule-S 追加
//! 一条补偿事实（saga）。补偿后可选立刻归档该事实（`finalize`），以保持域仍可终止。

use crate::client::ParticipantClient;
use crate::error::RuntimeError;
use async_trait::async_trait;
use pact_core::ArchiveEntry;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

/// 默认轮询兜底间隔。
pub const DEFAULT_EXECUTOR_POLL: Duration = Duration::from_millis(200);

/// 域发现重试次数与间隔（执行器不承担引导职责，可能先于引导者启动）。
const DISCOVERY_TRIES: usize = 1000;
const DISCOVERY_INTERVAL: Duration = Duration::from_millis(10);

/// 外部副作用处理器：真正落地外部动作（可能失败）。
#[async_trait]
pub trait SideEffectHandler: Send + Sync {
    /// 尝试落地某次消费对应的外部动作；`Err(msg)` 表示失败，触发补偿（或计入失败）。
    async fn apply(&self, entry: &ArchiveEntry) -> Result<(), String>;
}

/// 失败补偿：以 Rule-S 追加一条补偿事实 token。
#[derive(Debug, Clone)]
pub struct Compensation {
    /// 追加补偿事实的参与者（须对 `ty` 持有 seed 权）。
    pub participant: String,
    /// 补偿事实类型（源类型）。
    pub ty: String,
    /// 追加后立刻归档该事实（消费）。要求该参与者亦持有 `ty` 的 consume 门；
    /// 用于让补偿事实离开 Counter、保持域仍可终止。`false` 则事实留在 Counter。
    pub finalize: bool,
}

/// 执行器配置。
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    /// 执行器参与者名（用于补偿种子与归档）。
    pub participant: String,
    /// 触发外部动作的消费类型；空 = 全部。
    pub effect_types: Vec<String>,
    /// 失败补偿；`None` = 仅计数，不追加事实。
    pub compensation: Option<Compensation>,
    /// 轮询兜底间隔。
    pub poll: Duration,
}

impl ExecutorConfig {
    /// 以执行器参与者名构造。
    pub fn new(participant: impl Into<String>) -> Self {
        Self {
            participant: participant.into(),
            effect_types: Vec::new(),
            compensation: None,
            poll: DEFAULT_EXECUTOR_POLL,
        }
    }

    /// 设关注的消费类型集。
    pub fn watching(mut self, types: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.effect_types = types.into_iter().map(Into::into).collect();
        self
    }

    /// 设失败补偿。
    pub fn with_compensation(mut self, compensation: Compensation) -> Self {
        self.compensation = Some(compensation);
        self
    }

    /// 该归档条目是否触发外部动作。
    pub fn watches(&self, entry: &ArchiveEntry) -> bool {
        self.effect_types.is_empty() || self.effect_types.iter().any(|t| t == &entry.token.ty.0)
    }
}

/// 执行结果报告。
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ExecutorReport {
    /// 外部动作成功次数。
    pub applied: usize,
    /// 外部动作失败次数。
    pub failed: usize,
    /// 成功追加的补偿事实数。
    pub compensated: usize,
    /// 观察到 Rule-T 终止。
    pub saw_terminated: bool,
    /// 收到的推送通知条数。
    pub notifications: usize,
}

/// 停止信号：`stop()` 触发后执行器在下一次等待时退出（延迟上限为轮询间隔）。
#[derive(Clone, Default)]
pub struct StopSignal {
    flag: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl StopSignal {
    /// 新建。
    pub fn new() -> Self {
        Self::default()
    }

    /// 触发停止。
    pub fn stop(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// 是否已触发停止。
    pub fn is_stopped(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    async fn wait(&self) {
        if self.is_stopped() {
            return;
        }
        self.notify.notified().await;
    }
}

/// 外部副作用执行器：订阅 `Consumed`、落地外部动作、失败追加补偿事实。
pub struct SideEffectExecutor<C: ParticipantClient> {
    client: C,
    cfg: ExecutorConfig,
    /// 执行器自己可消费的类型集（用于 `finalize` 归档刚追加的补偿事实）。
    consumable: BTreeSet<String>,
}

impl<C: ParticipantClient> SideEffectExecutor<C> {
    /// 构造执行器（订阅与门查询在 [`Self::run`] 中惰性完成）。
    pub fn new(client: C, cfg: ExecutorConfig) -> Self {
        Self {
            client,
            cfg,
            consumable: BTreeSet::new(),
        }
    }

    /// 运行至 `stop` 触发或观察到 Rule-T 终止。
    ///
    /// 归档按条目序**游标增量**处理：每轮扫描只处理尚未处理的新条目，避免重复调用处理器，
    /// 也避免通知竞态造成的漏处理。
    pub async fn run(
        &mut self,
        handler: &impl SideEffectHandler,
        stop: &StopSignal,
    ) -> Result<ExecutorReport, RuntimeError> {
        // —— 引导：等待域出现（执行器不承担引导职责，可能先于引导者启动）——
        let domain = self.client.domain().to_owned();
        let mut found = false;
        for _ in 0..DISCOVERY_TRIES {
            if self
                .client
                .list_domains()
                .await?
                .iter()
                .any(|d| d == &domain)
            {
                found = true;
                break;
            }
            tokio::time::sleep(DISCOVERY_INTERVAL).await;
        }
        if !found {
            return Err(RuntimeError::Timeout(format!("执行器未发现域 {domain}")));
        }

        let mut wake = self.client.subscribe().await?;
        let gates = self.client.my_gates(&self.cfg.participant).await?;
        self.consumable = gates
            .iter()
            .filter(|g| g.action == pact_core::Action::Consume)
            .map(|g| g.ty.0.clone())
            .collect();

        let mut rep = ExecutorReport::default();
        let mut cursor = 0usize;

        loop {
            if stop.is_stopped() {
                break;
            }

            // —— 扫描新增归档条目（增量游标）——
            let entries = self.client.archive().await?;
            while cursor < entries.len() {
                let entry = &entries[cursor];
                if self.cfg.watches(entry) {
                    match handler.apply(entry).await {
                        Ok(()) => rep.applied += 1,
                        Err(_) => {
                            rep.failed += 1;
                            self.compensate(entry, &mut rep).await?;
                        }
                    }
                }
                cursor += 1;
            }

            // —— 终止判定：Counter 空且全部归档条目已处理（避免 pristine 误判）——
            let st = self.client.status().await?;
            if st.terminated && st.archive_size > 0 && cursor >= st.archive_size {
                rep.saw_terminated = true;
                break;
            }

            // —— 等待：推送优先，轮询兜底，停止信号 ——
            tokio::select! {
                n = wake.recv() => {
                    if n.is_some() {
                        rep.notifications += 1;
                    }
                }
                _ = tokio::time::sleep(self.cfg.poll) => {}
                _ = stop.wait() => break,
            }
        }

        self.client.close().await?;
        Ok(rep)
    }

    /// 追加补偿事实（Rule-S）；`finalize` 时立即归档之。
    async fn compensate(
        &self,
        entry: &ArchiveEntry,
        rep: &mut ExecutorReport,
    ) -> Result<(), RuntimeError> {
        let Some(c) = &self.cfg.compensation else {
            return Ok(());
        };
        let token_id = entry.token.id.0.clone();
        let seeded = self
            .client
            .seed(&c.participant, &c.ty, Some(&token_id))
            .await?;
        rep.compensated += 1;
        if c.finalize && self.consumable.contains(&c.ty) {
            self.client
                .consume(&c.participant, &seeded.id.0, None)
                .await?;
        }
        Ok(())
    }
}
