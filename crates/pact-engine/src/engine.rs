//! 引擎 actor（设计 §8）：tokio 任务 + mpsc 命令通道，独占持有全部域的
//! CounterState（D6 单权威）。每个命令处理 = 查找域 → 调用 pact-core 纯函数 →
//! 成功则 apply + 持久化 + 广播；检查与应用在同一临界区内原子完成。
//! 引擎只编排不判定（D14）：GateHolds、Rule-W、Rule-T 全部转调 core。

use crate::clock::Clock;
use crate::idgen::IdGen;
use crate::notify::{Notification, NotificationKind, NotificationSummary};
use crate::views::{
    AnalysisReport, ArchiveView, CompactionOutcome, CounterView, DomainListing, DomainSummary,
    GateClause, StatusView,
};
use pact_analysis as analysis;
use pact_compose::{compose as compose_fragments, Conflict};
use pact_core::{
    enabled_productions, rule_e, rule_p, rule_r, rule_s, termination_holds, ArchiveEntry,
    CounterState, Domain, Effect, ParticipantName, ProductionOption, RuleError, ThingRef, Token,
    TokenId, TypeName,
};
use pact_parser::{emit_domain, load_domain, Problem};
use pact_store::{apply_event, Event, InstanceKey, Snapshot, Store, StoreError};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot};

/// 默认实例名：接口层不指定实例时回退到该实例（FR19「不指定实例 ⇒ 默认实例」）。
pub const DEFAULT_INSTANCE: &str = "default";

// ============ 错误 ============

#[derive(Debug)]
pub enum EngineError {
    /// 实例未加载（内存会话与磁盘日志均无）。
    InstanceNotFound {
        domain: String,
        instance: String,
    },
    /// 域声明未登记（定义键缺失：未 declare）。
    DomainNotDeclared {
        name: String,
    },
    /// 目标实例已存在（不可重复 instantiate）。
    InstanceExists {
        domain: String,
        instance: String,
    },
    /// 目标实例非空且未显式 `force`（防止误替换销毁运行中的 Counter）。
    DomainBusy {
        domain: String,
        instance: String,
    },
    Store(StoreError),
    Parse(Problem),
    Rule(RuleError),
    Compose(Vec<Conflict>),
    /// 启动恢复失败（日志/声明损坏，需人工介入；后续全部命令以此失败）。
    Recovery(String),
    /// 幂等键冲突：同一 `request_id` 已用于另一 token 的消费。
    IdempotencyConflict {
        request_id: String,
        existing: TokenId,
        requested: TokenId,
    },
    /// 引擎已停止。
    EngineStopped,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::InstanceNotFound { domain, instance } => {
                write!(f, "实例未加载：{domain}/{instance}")
            }
            EngineError::DomainNotDeclared { name } => write!(f, "域声明未登记：{name}"),
            EngineError::InstanceExists { domain, instance } => {
                write!(f, "实例已存在：{domain}/{instance}")
            }
            EngineError::DomainBusy { domain, instance } => write!(
                f,
                "实例 {domain}/{instance} 非空（Reserved）：加载将重置其 Counter，\
                 如确认替换请显式 --force"
            ),
            EngineError::Store(e) => write!(f, "存储失败：{e}"),
            EngineError::Parse(e) => write!(f, "域声明无效：{e:?}"),
            EngineError::Rule(e) => write!(f, "规则拒绝：{e:?}"),
            EngineError::Compose(cs) => write!(f, "片段组合冲突：{cs:?}"),
            EngineError::Recovery(msg) => write!(f, "启动恢复失败：{msg}"),
            EngineError::IdempotencyConflict {
                request_id,
                existing,
                requested,
            } => write!(
                f,
                "幂等键 {request_id} 已用于消费 token {existing}，与本次请求的 {requested} 冲突"
            ),
            EngineError::EngineStopped => write!(f, "引擎已停止"),
        }
    }
}

impl std::error::Error for EngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EngineError::Store(e) => Some(e),
            _ => None,
        }
    }
}

impl From<StoreError> for EngineError {
    fn from(e: StoreError) -> Self {
        EngineError::Store(e)
    }
}

impl From<RuleError> for EngineError {
    fn from(e: RuleError) -> Self {
        EngineError::Rule(e)
    }
}

// ============ 配置 ============

#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// 每 N 个状态事件写一次快照（0 = 仅关停时写终态快照）。
    pub snapshot_every: u64,
    /// 每域实例 broadcast 通道容量（Rule-W 通知）。
    pub notify_capacity: usize,
    /// 命令通道容量。
    pub command_capacity: usize,
    /// 启动恢复后是否自动压缩全部实例日志（D25 子项 1；默认关闭，运维可显式触发）。
    pub compact_on_start: bool,
    /// 压缩时保留的最近事件条数（0 = 丢弃全部已入快照的前缀）。
    pub compact_keep_last: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            snapshot_every: 64,
            notify_capacity: 256,
            command_capacity: 256,
            compact_on_start: false,
            compact_keep_last: 0,
        }
    }
}

// ============ 命令 ============

type Responder<T> = oneshot::Sender<Result<T, EngineError>>;

pub enum Command {
    /// 声明 + 实例化默认/指定实例（declare + instantiate 的合并入口）。
    LoadDomain {
        name: String,
        declaration: String,
        instance: String,
        /// 目标实例非空时是否允许替换（默认 false → `DomainBusy`）。
        force: bool,
        resp: Responder<DomainSummary>,
    },
    /// 从已登记声明创建额外实例（不改变声明）。
    Instantiate {
        domain: String,
        instance: String,
        resp: Responder<DomainSummary>,
    },
    Validate {
        declaration: String,
        resp: Responder<Domain>,
    },
    Seed {
        key: InstanceKey,
        participant: ParticipantName,
        ty: TypeName,
        thing_ref: ThingRef,
        resp: Responder<Token>,
    },
    Produce {
        key: InstanceKey,
        participant: ParticipantName,
        ty: TypeName,
        predecessors: Vec<TokenId>,
        thing_ref: ThingRef,
        resp: Responder<Token>,
    },
    Read {
        key: InstanceKey,
        participant: ParticipantName,
        token_id: TokenId,
        resp: Responder<Token>,
    },
    Consume {
        key: InstanceKey,
        participant: ParticipantName,
        token_id: TokenId,
        /// 可选幂等键：同实例内重复请求返回首次结果（不重复消费）。
        request_id: Option<String>,
        resp: Responder<ArchiveEntry>,
    },
    QueryCounter {
        key: InstanceKey,
        filter: Option<TypeName>,
        resp: Responder<CounterView>,
    },
    QueryArchive {
        key: InstanceKey,
        filter: Option<TypeName>,
        resp: Responder<ArchiveView>,
    },
    QueryGates {
        key: InstanceKey,
        participant: ParticipantName,
        resp: Responder<Vec<GateClause>>,
    },
    QueryEnabled {
        key: InstanceKey,
        participant: ParticipantName,
        resp: Responder<Vec<ProductionOption>>,
    },
    QueryStatus {
        key: InstanceKey,
        resp: Responder<StatusView>,
    },
    ListDomains {
        resp: Responder<Vec<DomainListing>>,
    },
    /// 卸载实例（内存态；日志保留，设计 §9.2 DELETE）。
    UnloadDomain {
        key: InstanceKey,
        resp: Responder<()>,
    },
    Subscribe {
        key: InstanceKey,
        resp: Responder<broadcast::Receiver<Notification>>,
    },
    Analyze {
        text: String,
        resp: Responder<AnalysisReport>,
    },
    /// 分析已加载域**声明**（`POST /domains/{d}:analyze`，定义级）。
    AnalyzeLoaded {
        domain: String,
        resp: Responder<AnalysisReport>,
    },
    ComposeAndLoad {
        name: String,
        fragments: Vec<String>,
        instance: String,
        resp: Responder<DomainSummary>,
    },
    /// 日志前缀压缩（D25 子项 1）：写全量快照后截断已覆盖前缀。
    Compact {
        key: InstanceKey,
        /// 保留的最近事件条数（0 = 清空已入快照的前缀）。
        keep_last: u64,
        resp: Responder<CompactionOutcome>,
    },
    Shutdown {
        resp: Responder<()>,
    },
}

impl Command {
    /// 以统一错误应答命令（启动恢复失败时排空命令队列用）。
    /// 各命令 responder 类型不同，无法合并 or-pattern，逐变体应答。
    fn fail(self, err: EngineError) {
        match self {
            Command::LoadDomain { resp, .. }
            | Command::ComposeAndLoad { resp, .. }
            | Command::Instantiate { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::Validate { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::Seed { resp, .. }
            | Command::Produce { resp, .. }
            | Command::Read { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::Consume { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::QueryCounter { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::QueryArchive { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::QueryGates { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::QueryEnabled { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::QueryStatus { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::ListDomains { resp } => {
                let _ = resp.send(Err(err));
            }
            Command::UnloadDomain { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::Subscribe { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::Analyze { resp, .. } | Command::AnalyzeLoaded { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::Compact { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::Shutdown { resp } => {
                let _ = resp.send(Err(err));
            }
        }
    }
}

// ============ 会话 ============

/// 声明注册表条目（**定义键** `domain`）：解析后的声明 + 原文。
struct DeclarationEntry {
    declaration: String,
    domain: Domain,
}

/// 实例会话（**实例键** `InstanceKey`）；`declaration` = 实例创建时的声明快照。
struct InstanceSession {
    /// 实例创建（或最近一次域重置）时的声明原文。日志前缀被压缩后，
    /// 该字段随快照持久化，成为重放恢复的唯一声明来源（D25 子项 1）。
    declaration: String,
    domain: Domain,
    state: CounterState,
    last_seq: u64,
    /// 本实例最近一次 `DomainLoaded` 的 seq（「实例非空」判据基线，R5）。
    dl_seq: u64,
    events_since_snapshot: u64,
    /// 当前 Rule-T 判定（信息性，不锁定域实例——种子仍可注入）。
    terminated: bool,
    /// Rule-E 幂等表：request_id → (目标 token id, 首次归档结果)。
    /// 随事件日志持久化（Consumed 事件的 request_id 字段），启动时重建。
    consumed_requests: BTreeMap<String, (TokenId, ArchiveEntry)>,
    notify: broadcast::Sender<Notification>,
}

impl InstanceSession {
    /// 「实例非空」：最后一次 DomainLoaded 之后已有事件（R5）。
    fn is_occupied(&self) -> bool {
        self.last_seq > self.dl_seq
    }
}

// ============ Actor ============

struct EngineActor {
    store: Store,
    clock: Arc<dyn Clock>,
    idgen: Arc<dyn IdGen>,
    config: EngineConfig,
    /// 定义键 → 声明（跨实例共享；declare 登记）。
    declarations: BTreeMap<String, DeclarationEntry>,
    /// 实例键 → 运行会话。
    instances: BTreeMap<InstanceKey, InstanceSession>,
}

impl EngineActor {
    async fn run(mut self, mut rx: mpsc::Receiver<Command>) {
        if let Err(e) = self.recover().await {
            // 恢复失败：排空队列并以统一错误应答（fail-fast，需人工介入）
            let msg = e.to_string();
            while let Some(cmd) = rx.recv().await {
                cmd.fail(EngineError::Recovery(msg.clone()));
            }
            return;
        }
        while let Some(cmd) = rx.recv().await {
            if self.handle(cmd).await {
                break; // Shutdown
            }
        }
    }

    /// 启动恢复：store 清洗日志 + 索引校验 → 回填声明注册表 → 逐域实例重建会话。
    async fn recover(&mut self) -> Result<(), EngineError> {
        let recovered = self.store.recover().await?;

        // 声明注册表回填（定义键与实例解耦）：优先 `<domain>/declaration.pact`，
        // 缺失则回退该域 `default` 实例的声明快照，再退化为按 (domain, instance) 排序首个实例。
        let mut by_domain: BTreeMap<String, String> = BTreeMap::new();
        for ri in &recovered {
            if ri.key.instance == DEFAULT_INSTANCE {
                if let Some(d) = last_declaration(&ri.events) {
                    by_domain.insert(ri.key.domain.clone(), d);
                }
            }
        }
        for ri in &recovered {
            if let Some(d) = last_declaration(&ri.events) {
                by_domain.entry(ri.key.domain.clone()).or_insert(d);
            }
        }
        for (name, fallback) in by_domain {
            let declaration = match self.store.load_declaration(&name)? {
                Some(d) => d,
                None => {
                    eprintln!(
                        "pactd: 域 {name} 缺 declaration.pact，回退用实例声明快照（建议人工补写）"
                    );
                    fallback
                }
            };
            let domain = load_domain(&declaration).map_err(EngineError::Parse)?;
            self.declarations.insert(
                name,
                DeclarationEntry {
                    declaration,
                    domain,
                },
            );
        }

        for ri in recovered {
            self.recover_instance(ri)?;
        }

        // 可选的启动自动压缩（D25 子项 1，默认关闭）：以当前状态写真快照并截断已覆盖前缀。
        // 失败不阻断启动（压缩是空间优化而非正确性前提），仅告警。
        if self.config.compact_on_start {
            let keys: Vec<InstanceKey> = self.instances.keys().cloned().collect();
            let keep_last = self.config.compact_keep_last;
            for key in keys {
                if let Err(e) = self.compact(&key, keep_last).await {
                    eprintln!("pactd: 启动自动压缩实例 {key} 失败（跳过）：{e}");
                }
            }
        }
        Ok(())
    }

    /// 单实例恢复：优先用日志最后一个 `DomainLoaded` 解析声明；日志前缀被压缩
    /// （无 `DomainLoaded`）时改用快照锚（快照承载声明原文、域重置基线与幂等表）→
    /// 快照快速路径（`snapshot.seq ≥ DomainLoaded.seq` 且不超日志末尾才有效）→ 尾部重放。
    fn recover_instance(&mut self, ri: pact_store::RecoveredInstance) -> Result<(), EngineError> {
        let key = ri.key;
        let events = ri.events;
        let snap = self.store.load_snapshot(&key)?;
        let snap_seq = snap.as_ref().map(|s| s.seq).unwrap_or(0);
        // 续接点：日志末尾 / 压缩水位 / 快照三者最大（压缩后日志可能为空）
        let last_seq = events
            .last()
            .map(|e| e.seq())
            .unwrap_or(0)
            .max(ri.base_seq)
            .max(snap_seq);

        let dl_idx = events
            .iter()
            .rposition(|e| matches!(e, Event::DomainLoaded { .. }));

        // 声明与域重置基线
        let (declaration, dl_seq, snapshot_is_anchor) = match dl_idx {
            Some(idx) => match &events[idx] {
                Event::DomainLoaded {
                    declaration, seq, ..
                } => (declaration.clone(), *seq, false),
                _ => unreachable!(),
            },
            // 前缀已被压缩：日志已无 DomainLoaded，快照是唯一的历史载体
            None => match &snap {
                Some(s) => (s.declaration.clone(), s.dl_seq, true),
                None => {
                    return Err(EngineError::Recovery(format!(
                        "实例 {key} 的日志无 DomainLoaded 且无可用快照：历史状态不可恢复"
                    )))
                }
            },
        };
        let domain = load_domain(&declaration).map_err(EngineError::Parse)?;

        let (mut state, base_seq) = match (&snap, snapshot_is_anchor) {
            // 快照锚：状态直接取快照（压缩已丢弃其后之前缀）
            (Some(s), true) => (s.state.clone(), s.seq),
            // 快照快速路径：须晚于最后一次域重置且不超日志末尾
            (Some(s), false) if s.seq >= dl_seq && s.seq <= last_seq => (s.state.clone(), s.seq),
            _ => (CounterState::new(), dl_seq),
        };
        for ev in events.iter().filter(|e| e.seq() > base_seq) {
            apply_event(&mut state, ev)?;
        }

        // Rule-E 幂等表：
        // - 日志有 DomainLoaded（未压缩或重置后）：从该重置之后的全部 Consumed 事件重建；
        // - 快照锚（前缀已压缩）：以快照承载的表为基，补上快照之后的事件。
        let idem_floor = if snapshot_is_anchor { base_seq } else { dl_seq };
        let from_log = events
            .iter()
            .filter(|e| e.seq() > idem_floor)
            .filter_map(|e| match e {
                Event::Consumed {
                    token,
                    by,
                    ts,
                    request_id: Some(rid),
                    ..
                } => Some((
                    rid.clone(),
                    (
                        token.id.clone(),
                        ArchiveEntry {
                            token: token.clone(),
                            exhausted_by: by.clone(),
                            exhausted_at: ts.clone(),
                        },
                    ),
                )),
                _ => None,
            });
        let mut consumed_requests: BTreeMap<String, (TokenId, ArchiveEntry)> = if snapshot_is_anchor
        {
            snap.as_ref()
                .map(|s| s.consumed_requests.clone())
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        for (rid, entry) in from_log {
            consumed_requests.insert(rid, entry);
        }

        let terminated = termination_holds(&domain, &state);
        let (notify, _) = broadcast::channel(self.config.notify_capacity);
        self.instances.insert(
            key,
            InstanceSession {
                declaration,
                domain,
                state,
                last_seq,
                dl_seq,
                events_since_snapshot: 0,
                terminated,
                consumed_requests,
                notify,
            },
        );
        Ok(())
    }

    /// 处理一条命令。返回 true 表示 Shutdown（退出 actor）。
    async fn handle(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::LoadDomain {
                name,
                declaration,
                instance,
                force,
                resp,
            } => respond(
                resp,
                self.load_domain(&name, &declaration, &instance, force)
                    .await,
            ),
            Command::Instantiate {
                domain,
                instance,
                resp,
            } => respond(resp, self.instantiate(&domain, &instance).await),
            Command::Validate { declaration, resp } => {
                respond(resp, load_domain(&declaration).map_err(EngineError::Parse))
            }
            Command::Seed {
                key,
                participant,
                ty,
                thing_ref,
                resp,
            } => respond(resp, self.seed(&key, &participant, &ty, thing_ref).await),
            Command::Produce {
                key,
                participant,
                ty,
                predecessors,
                thing_ref,
                resp,
            } => respond(
                resp,
                self.produce(&key, &participant, &ty, &predecessors, thing_ref)
                    .await,
            ),
            Command::Read {
                key,
                participant,
                token_id,
                resp,
            } => respond(resp, self.read(&key, &participant, &token_id).await),
            Command::Consume {
                key,
                participant,
                token_id,
                request_id,
                resp,
            } => respond(
                resp,
                self.consume(&key, &participant, &token_id, request_id)
                    .await,
            ),
            Command::QueryCounter { key, filter, resp } => {
                respond(resp, self.query_counter(&key, filter.as_ref()))
            }
            Command::QueryArchive { key, filter, resp } => {
                respond(resp, self.query_archive(&key, filter.as_ref()))
            }
            Command::QueryGates {
                key,
                participant,
                resp,
            } => respond(resp, self.query_gates(&key, &participant)),
            Command::QueryEnabled {
                key,
                participant,
                resp,
            } => respond(resp, self.query_enabled(&key, &participant)),
            Command::QueryStatus { key, resp } => respond(resp, self.query_status(&key)),
            Command::ListDomains { resp } => respond(resp, Ok(self.list_domains())),
            Command::UnloadDomain { key, resp } => respond(resp, self.unload_domain(&key)),
            Command::Subscribe { key, resp } => respond(resp, self.subscribe(&key)),
            Command::Analyze { text, resp } => respond(resp, self.analyze(&text)),
            Command::AnalyzeLoaded { domain, resp } => respond(resp, self.analyze_loaded(&domain)),
            Command::ComposeAndLoad {
                name,
                fragments,
                instance,
                resp,
            } => respond(
                resp,
                self.compose_and_load(&name, &fragments, &instance).await,
            ),
            Command::Compact {
                key,
                keep_last,
                resp,
            } => respond(resp, self.compact(&key, keep_last).await),
            Command::Shutdown { resp } => {
                respond(resp, self.snapshot_all());
                return true;
            }
        }
        false
    }

    // —— 域/实例生命周期 ——

    /// 列出定义键及其实例（`GET /domains`）：以**运行中实例**为准，
    /// 并合并磁盘上存在但未加载的实例（避免遗漏）。
    fn list_domains(&self) -> Vec<DomainListing> {
        let mut listings: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for key in self.store.list_instances() {
            listings.entry(key.domain).or_default().push(key.instance);
        }
        for key in self.instances.keys() {
            listings
                .entry(key.domain.clone())
                .or_default()
                .push(key.instance.clone());
        }
        listings
            .into_iter()
            .map(|(name, mut instances)| {
                instances.sort();
                instances.dedup();
                DomainListing { name, instances }
            })
            .collect()
    }

    /// 登记声明（定义键）：解析 + 落 `declaration.pact`（best-effort）+ 注册表覆盖。
    /// 已运行实例的声明快照不受影响（重放自封闭）。
    fn declare(&mut self, name: &str, declaration: &str) -> Result<(), EngineError> {
        pact_store::validate_domain_name(name)?;
        let domain = load_domain(declaration).map_err(EngineError::Parse)?;
        if let Err(e) = self.store.write_declaration(name, declaration) {
            eprintln!("pactd: 写 declaration.pact 失败（域 {name}）：{e}");
        }
        self.declarations.insert(
            name.to_owned(),
            DeclarationEntry {
                declaration: declaration.to_owned(),
                domain,
            },
        );
        Ok(())
    }

    /// 实例化：从已登记声明创建实例（不改变声明）。新实例追加 `DomainLoaded`，
    /// seq 依据存储在实例日志上的续接点（兼容"卸载后再实例化"）。
    async fn instantiate(
        &mut self,
        name: &str,
        instance: &str,
    ) -> Result<DomainSummary, EngineError> {
        pact_store::validate_domain_name(name)?;
        pact_store::validate_instance_name(instance)?;
        let key = InstanceKey::new(name, instance);
        if self.instances.contains_key(&key) {
            return Err(EngineError::InstanceExists {
                domain: name.to_owned(),
                instance: instance.to_owned(),
            });
        }
        let entry = self
            .declarations
            .get(name)
            .ok_or_else(|| EngineError::DomainNotDeclared {
                name: name.to_owned(),
            })?;
        let declaration = entry.declaration.clone();
        let domain = entry.domain.clone();
        let next_seq = self.store.last_seq(&key) + 1;
        let ev = Event::DomainLoaded {
            seq: next_seq,
            ts: self.clock.now(),
            name: name.to_owned(),
            declaration: declaration.clone(),
        };
        self.store.append(&key, &ev).await?;
        let summary = DomainSummary {
            name: name.to_owned(),
            instance: instance.to_owned(),
            types: domain.types.len(),
            gates: domain.gates.len(),
            seed_authority: domain.seed_authority.len(),
            last_seq: next_seq,
        };
        let terminated = termination_holds(&domain, &CounterState::new());
        let (notify, _) = broadcast::channel(self.config.notify_capacity);
        self.instances.insert(
            key,
            InstanceSession {
                declaration,
                domain,
                state: CounterState::new(),
                last_seq: next_seq,
                dl_seq: next_seq,
                events_since_snapshot: 0,
                terminated,
                consumed_requests: BTreeMap::new(),
                notify,
            },
        );
        Ok(summary)
    }

    /// `LoadDomain` = declare + instantiate：登记声明后创建/重置目标实例。
    /// 目标实例**非空且未 `force`** → `DomainBusy`（防误替换销毁运行中 Counter）。
    async fn load_domain(
        &mut self,
        name: &str,
        declaration: &str,
        instance: &str,
        force: bool,
    ) -> Result<DomainSummary, EngineError> {
        pact_store::validate_domain_name(name)?;
        pact_store::validate_instance_name(instance)?;
        self.declare(name, declaration)?;

        let key = InstanceKey::new(name, instance);
        match self.instances.get(&key) {
            // 非空且未显式 force：拒绝（Reserved 实例的 Counter 将随重置被销毁）
            Some(s) if s.is_occupied() && !force => Err(EngineError::DomainBusy {
                domain: name.to_owned(),
                instance: instance.to_owned(),
            }),
            // 已存在（空或已 force）：重置语义 —— 清快照、状态清零、seq 续接
            Some(s) => {
                let next_seq = s.last_seq + 1;
                let ev = Event::DomainLoaded {
                    seq: next_seq,
                    ts: self.clock.now(),
                    name: name.to_owned(),
                    declaration: declaration.to_owned(),
                };
                self.store.append(&key, &ev).await?;
                let _ = self.store.remove_snapshot(&key);
                let domain = load_domain(declaration).map_err(EngineError::Parse)?;
                let summary = DomainSummary {
                    name: name.to_owned(),
                    instance: instance.to_owned(),
                    types: domain.types.len(),
                    gates: domain.gates.len(),
                    seed_authority: domain.seed_authority.len(),
                    last_seq: next_seq,
                };
                let terminated = termination_holds(&domain, &CounterState::new());
                let (notify, _) = broadcast::channel(self.config.notify_capacity);
                self.instances.insert(
                    key,
                    InstanceSession {
                        declaration: declaration.to_owned(),
                        domain,
                        state: CounterState::new(),
                        last_seq: next_seq,
                        dl_seq: next_seq,
                        events_since_snapshot: 0,
                        terminated,
                        consumed_requests: BTreeMap::new(),
                        notify,
                    },
                );
                Ok(summary)
            }
            // 新实例
            None => self.instantiate(name, instance).await,
        }
    }

    async fn compose_and_load(
        &mut self,
        name: &str,
        fragments: &[String],
        instance: &str,
    ) -> Result<DomainSummary, EngineError> {
        let mut domains = Vec::with_capacity(fragments.len());
        for f in fragments {
            domains.push(load_domain(f).map_err(EngineError::Parse)?);
        }
        let merged = compose_fragments(&domains).map_err(EngineError::Compose)?;
        let declaration = emit_domain(&merged);
        self.load_domain(name, &declaration, instance, false).await
    }

    // —— 四类动作（Rule-S/P/R/E）——

    async fn seed(
        &mut self,
        key: &InstanceKey,
        participant: &ParticipantName,
        ty: &TypeName,
        thing_ref: ThingRef,
    ) -> Result<Token, EngineError> {
        let id = self.idgen.next();
        let now = self.clock.now();
        // 直接字段访问：instances 的可变借用与 store/clock/config 的使用不相交
        let session = self
            .instances
            .get_mut(key)
            .ok_or_else(|| instance_not_found(key))?;
        let token = rule_s(&session.domain, participant, ty, thing_ref, id, now.clone())?;
        let seq = session.last_seq + 1;
        let ev = Event::Seeded {
            seq,
            ts: now.clone(),
            token: token.clone(),
        };
        self.store.append(key, &ev).await?;
        session.state.apply(Effect::Seeded(token.clone()));
        session.last_seq = seq;
        notify_state_change(
            session,
            key,
            NotificationKind::Seeded,
            seq,
            &now,
            Some(token.id.clone()),
        );
        post_commit(&self.store, self.clock.as_ref(), &self.config, key, session).await;
        Ok(token)
    }

    async fn produce(
        &mut self,
        key: &InstanceKey,
        participant: &ParticipantName,
        ty: &TypeName,
        predecessors: &[TokenId],
        thing_ref: ThingRef,
    ) -> Result<Token, EngineError> {
        let id = self.idgen.next();
        let now = self.clock.now();
        let session = self
            .instances
            .get_mut(key)
            .ok_or_else(|| instance_not_found(key))?;
        let token = rule_p(
            &session.domain,
            &session.state,
            participant,
            ty,
            predecessors,
            thing_ref,
            id,
            now.clone(),
        )?;
        let seq = session.last_seq + 1;
        let ev = Event::Produced {
            seq,
            ts: now.clone(),
            token: token.clone(),
        };
        self.store.append(key, &ev).await?;
        session.state.apply(Effect::Produced(token.clone()));
        session.last_seq = seq;
        notify_state_change(
            session,
            key,
            NotificationKind::Produced,
            seq,
            &now,
            Some(token.id.clone()),
        );
        post_commit(&self.store, self.clock.as_ref(), &self.config, key, session).await;
        Ok(token)
    }

    /// Rule-R：无状态变更、无事件（D9）、无通知。
    async fn read(
        &mut self,
        key: &InstanceKey,
        participant: &ParticipantName,
        token_id: &TokenId,
    ) -> Result<Token, EngineError> {
        let session = self.session(key)?;
        rule_r(&session.domain, &session.state, participant, token_id).map_err(EngineError::Rule)
    }

    async fn consume(
        &mut self,
        key: &InstanceKey,
        participant: &ParticipantName,
        token_id: &TokenId,
        request_id: Option<String>,
    ) -> Result<ArchiveEntry, EngineError> {
        let now = self.clock.now();
        let session = self
            .instances
            .get_mut(key)
            .ok_or_else(|| instance_not_found(key))?;
        // 幂等命中：同一 request_id 的重复请求直接返回首次结果，不重复消费。
        // 首次结果随 Consumed 事件持久化，跨重启仍生效（恢复时重建幂等表）。
        if let Some(rid) = &request_id {
            if let Some((existing, entry)) = session.consumed_requests.get(rid) {
                if existing == token_id {
                    return Ok(entry.clone());
                }
                return Err(EngineError::IdempotencyConflict {
                    request_id: rid.clone(),
                    existing: existing.clone(),
                    requested: token_id.clone(),
                });
            }
        }
        let entry = rule_e(
            &session.domain,
            &session.state,
            participant,
            token_id,
            now.clone(),
        )?;
        let seq = session.last_seq + 1;
        let ev = Event::Consumed {
            seq,
            ts: now.clone(),
            token: entry.token.clone(),
            by: participant.clone(),
            request_id: request_id.clone(),
        };
        // 提交点 = 日志落盘（store.append 内部 fsync）。其后步骤均不可失败（见 post_commit）。
        self.store.append(key, &ev).await?;
        session.state.apply(Effect::Consumed(entry.clone()));
        session.last_seq = seq;
        if let Some(rid) = request_id {
            session
                .consumed_requests
                .insert(rid, (token_id.clone(), entry.clone()));
        }
        notify_state_change(
            session,
            key,
            NotificationKind::Consumed,
            seq,
            &now,
            Some(token_id.clone()),
        );
        post_commit(&self.store, self.clock.as_ref(), &self.config, key, session).await;
        Ok(entry)
    }

    // —— 查询 ——

    fn query_counter(
        &self,
        key: &InstanceKey,
        filter: Option<&TypeName>,
    ) -> Result<CounterView, EngineError> {
        let s = self.session(key)?;
        let tokens: Vec<Token> = s
            .state
            .counter
            .values()
            .filter(|t| filter.is_none_or(|f| t.ty == *f))
            .cloned()
            .collect();
        let size = tokens.len();
        Ok(CounterView { tokens, size })
    }

    fn query_archive(
        &self,
        key: &InstanceKey,
        filter: Option<&TypeName>,
    ) -> Result<ArchiveView, EngineError> {
        let s = self.session(key)?;
        let entries: Vec<ArchiveEntry> = s
            .state
            .archive
            .iter()
            .filter(|e| filter.is_none_or(|f| e.token.ty == *f))
            .cloned()
            .collect();
        let size = entries.len();
        Ok(ArchiveView { entries, size })
    }

    fn query_gates(
        &self,
        key: &InstanceKey,
        participant: &ParticipantName,
    ) -> Result<Vec<GateClause>, EngineError> {
        let s = self.session(key)?;
        Ok(s.domain
            .gates
            .iter()
            .filter(|((p, _, _), _)| p == participant)
            .map(|((_, action, ty), cond)| GateClause {
                action: *action,
                ty: ty.clone(),
                condition: cond.clone(),
            })
            .collect())
    }

    fn query_enabled(
        &self,
        key: &InstanceKey,
        participant: &ParticipantName,
    ) -> Result<Vec<ProductionOption>, EngineError> {
        let s = self.session(key)?;
        Ok(enabled_productions(&s.domain, &s.state, participant))
    }

    fn query_status(&self, key: &InstanceKey) -> Result<StatusView, EngineError> {
        let s = self.session(key)?;
        Ok(StatusView {
            counter_size: s.state.counter.len(),
            archive_size: s.state.archive.len(),
            terminated: s.terminated,
        })
    }

    fn subscribe(
        &self,
        key: &InstanceKey,
    ) -> Result<broadcast::Receiver<Notification>, EngineError> {
        Ok(self.session(key)?.notify.subscribe())
    }

    fn analyze(&self, text: &str) -> Result<AnalysisReport, EngineError> {
        let domain = load_domain(text).map_err(EngineError::Parse)?;
        Ok(report_of(&domain))
    }

    /// 分析已登记声明（定义级，无需实例）。
    fn analyze_loaded(&self, name: &str) -> Result<AnalysisReport, EngineError> {
        let entry = self
            .declarations
            .get(name)
            .ok_or_else(|| domain_not_declared(name))?;
        Ok(report_of(&entry.domain))
    }

    /// 卸载实例：仅移除内存会话；日志/快照保留，重启后自动恢复（设计 §9.2）。
    fn unload_domain(&mut self, key: &InstanceKey) -> Result<(), EngineError> {
        if self.instances.remove(key).is_none() {
            return Err(instance_not_found(key));
        }
        Ok(())
    }

    // —— 内部 ——

    fn session(&self, key: &InstanceKey) -> Result<&InstanceSession, EngineError> {
        self.instances
            .get(key)
            .ok_or_else(|| instance_not_found(key))
    }

    /// 日志前缀压缩（D25 子项 1）。位于 actor 临界区：与其它写命令串行，不与之竞争。
    ///
    /// 顺序即正确性：先以**当前状态**写全量自足快照（提交点），再截断日志前缀；
    /// 崩溃落在任一步之间，恢复都只需快照 + 残留日志即可还原。
    async fn compact(
        &mut self,
        key: &InstanceKey,
        keep_last: u64,
    ) -> Result<CompactionOutcome, EngineError> {
        let snap = {
            let session = self.session(key)?;
            snapshot_of(session)
        };
        let ts = self.clock.now();
        let res = self.store.compact(key, &snap, keep_last, &ts).await?;
        if let Some(session) = self.instances.get_mut(key) {
            // 快照已覆盖到日志末尾 → 重置定期快照计数
            session.events_since_snapshot = 0;
        }
        Ok(CompactionOutcome {
            domain: key.domain.clone(),
            instance: key.instance.clone(),
            snapshot_seq: res.snapshot_seq,
            base_seq: res.base_seq,
            retained_events: res.retained,
            discarded_events: res.discarded,
        })
    }

    /// 关停前终态快照（全部实例）。
    fn snapshot_all(&self) -> Result<(), EngineError> {
        for (key, s) in &self.instances {
            self.store.write_snapshot(key, &snapshot_of(s))?;
        }
        Ok(())
    }
}

/// 实例当前状态的**完整自足快照**：状态 + 声明快照 + 域重置基线 + Rule-E 幂等表。
///
/// 压缩后快照是被丢弃前缀的唯一载体，故所有快照（定期、终态、压缩）都必须自足，
/// 否则日志前缀被截断后无法恢复幂等表（D25 子项 1）。
fn snapshot_of(session: &InstanceSession) -> Snapshot {
    Snapshot::new(
        session.last_seq,
        session.dl_seq,
        session.declaration.clone(),
        session.state.clone(),
        session.consumed_requests.clone(),
    )
}

/// 状态变更通知（§6.4）：变更后 Counter 规模随通知发出。
fn notify_state_change(
    session: &InstanceSession,
    key: &InstanceKey,
    kind: NotificationKind,
    seq: u64,
    ts: &str,
    token_id: Option<TokenId>,
) {
    let _ = session.notify.send(Notification {
        version: pact_store::EVENT_SCHEMA_VERSION,
        event: kind,
        domain: key.domain.clone(),
        instance: key.instance.clone(),
        seq,
        ts: ts.to_owned(),
        token_id,
        summary: NotificationSummary {
            counter_size: session.state.counter.len(),
        },
    });
}

/// 状态变更后的公共提交尾部：定期快照 + Rule-T 惰性求值。
/// 二者均位于提交点（日志落盘）之后，为**尽力而为的派生步骤、不可失败**：
/// - 快照失败：正确性由重放兜底（设计 §7.4），跳过本次并保留计数待下次重试；
/// - Terminated 事件是可由状态重导出的信息，追加失败不影响已提交命令，重启时重算。
async fn post_commit(
    store: &Store,
    clock: &dyn Clock,
    config: &EngineConfig,
    key: &InstanceKey,
    session: &mut InstanceSession,
) {
    // 定期快照（尽力而为）
    session.events_since_snapshot += 1;
    if config.snapshot_every > 0
        && session.events_since_snapshot >= config.snapshot_every
        && store.write_snapshot(key, &snapshot_of(session)).is_ok()
    {
        session.events_since_snapshot = 0;
    }

    // Rule-T 惰性求值（D14：转调 core）
    let now_terminated = termination_holds(&session.domain, &session.state);
    if now_terminated && !session.terminated {
        let seq = session.last_seq + 1;
        let ts = clock.now();
        if store
            .append(
                key,
                &Event::Terminated {
                    seq,
                    ts: ts.clone(),
                },
            )
            .await
            .is_ok()
        {
            session.last_seq = seq;
            let _ = session.notify.send(Notification {
                version: pact_store::EVENT_SCHEMA_VERSION,
                event: NotificationKind::Terminated,
                domain: key.domain.clone(),
                instance: key.instance.clone(),
                seq,
                ts,
                token_id: None,
                summary: NotificationSummary {
                    counter_size: session.state.counter.len(),
                },
            });
        }
    }
    session.terminated = now_terminated;
}

/// 实例最后一次 `DomainLoaded` 的声明原文（恢复回填用）。
fn last_declaration(events: &[Event]) -> Option<String> {
    events.iter().rev().find_map(|e| match e {
        Event::DomainLoaded { declaration, .. } => Some(declaration.clone()),
        _ => None,
    })
}

fn respond<T>(resp: oneshot::Sender<Result<T, EngineError>>, result: Result<T, EngineError>) {
    let _ = resp.send(result);
}

/// 六项静态分析聚合（论文 Appendix A.1–A.6）。
fn report_of(domain: &Domain) -> AnalysisReport {
    AnalysisReport {
        // A.5 先算（Appendix D：它是其余分析的前提）
        precondition_wf: analysis::precondition_wf(domain),
        reachability: analysis::reachability(domain),
        gate_coverage: analysis::gate_coverage(domain),
        consumption_reachability: analysis::consumption_reachability(domain),
        structural_deadlock: analysis::structural_deadlock(domain),
        stranding_risk: analysis::stranding_risk(domain),
    }
}

fn instance_not_found(key: &InstanceKey) -> EngineError {
    EngineError::InstanceNotFound {
        domain: key.domain.clone(),
        instance: key.instance.clone(),
    }
}

fn domain_not_declared(name: &str) -> EngineError {
    EngineError::DomainNotDeclared {
        name: name.to_owned(),
    }
}

// ============ 引擎门面（句柄） ============

/// 引擎句柄：Clone 共享；所有方法经命令通道串行执行（D6）。
#[derive(Clone)]
pub struct Engine {
    tx: mpsc::Sender<Command>,
}

impl Engine {
    /// 以默认配置启动引擎 actor（含启动恢复）。
    pub fn spawn(store: Store, clock: Arc<dyn Clock>, idgen: Arc<dyn IdGen>) -> Self {
        Self::spawn_with_config(store, clock, idgen, EngineConfig::default())
    }

    pub fn spawn_with_config(
        store: Store,
        clock: Arc<dyn Clock>,
        idgen: Arc<dyn IdGen>,
        config: EngineConfig,
    ) -> Self {
        let (tx, rx) = mpsc::channel(config.command_capacity);
        let actor = EngineActor {
            store,
            clock,
            idgen,
            config,
            declarations: BTreeMap::new(),
            instances: BTreeMap::new(),
        };
        tokio::spawn(actor.run(rx));
        Self { tx }
    }

    /// 发送原始命令（接口层可直接映射）。
    pub async fn execute(&self, cmd: Command) -> Result<(), EngineError> {
        self.tx
            .send(cmd)
            .await
            .map_err(|_| EngineError::EngineStopped)
    }

    /// `LoadDomain` = declare + instantiate：登记声明后创建/重置目标实例。
    pub async fn load_domain(
        &self,
        name: &str,
        declaration: &str,
        instance: &str,
        force: bool,
    ) -> Result<DomainSummary, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::LoadDomain {
            name: name.to_owned(),
            declaration: declaration.to_owned(),
            instance: instance.to_owned(),
            force,
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    /// 从已登记声明创建额外实例。
    pub async fn instantiate(
        &self,
        domain: &str,
        instance: &str,
    ) -> Result<DomainSummary, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Instantiate {
            domain: domain.to_owned(),
            instance: instance.to_owned(),
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn validate(&self, declaration: &str) -> Result<Domain, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Validate {
            declaration: declaration.to_owned(),
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn seed(
        &self,
        key: InstanceKey,
        participant: ParticipantName,
        ty: TypeName,
        thing_ref: ThingRef,
    ) -> Result<Token, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Seed {
            key,
            participant,
            ty,
            thing_ref,
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn produce(
        &self,
        key: InstanceKey,
        participant: ParticipantName,
        ty: TypeName,
        predecessors: Vec<TokenId>,
        thing_ref: ThingRef,
    ) -> Result<Token, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Produce {
            key,
            participant,
            ty,
            predecessors,
            thing_ref,
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn read(
        &self,
        key: InstanceKey,
        participant: ParticipantName,
        token_id: TokenId,
    ) -> Result<Token, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Read {
            key,
            participant,
            token_id,
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn consume(
        &self,
        key: InstanceKey,
        participant: ParticipantName,
        token_id: TokenId,
        request_id: Option<String>,
    ) -> Result<ArchiveEntry, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Consume {
            key,
            participant,
            token_id,
            request_id,
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn counter(
        &self,
        key: InstanceKey,
        filter: Option<TypeName>,
    ) -> Result<CounterView, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::QueryCounter { key, filter, resp })
            .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn archive(
        &self,
        key: InstanceKey,
        filter: Option<TypeName>,
    ) -> Result<ArchiveView, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::QueryArchive { key, filter, resp })
            .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn my_gates(
        &self,
        key: InstanceKey,
        participant: ParticipantName,
    ) -> Result<Vec<GateClause>, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::QueryGates {
            key,
            participant,
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn enabled(
        &self,
        key: InstanceKey,
        participant: ParticipantName,
    ) -> Result<Vec<ProductionOption>, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::QueryEnabled {
            key,
            participant,
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn status(&self, key: InstanceKey) -> Result<StatusView, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::QueryStatus { key, resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn list_domains(&self) -> Result<Vec<DomainListing>, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::ListDomains { resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn subscribe(
        &self,
        key: InstanceKey,
    ) -> Result<broadcast::Receiver<Notification>, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Subscribe { key, resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn analyze(&self, text: &str) -> Result<AnalysisReport, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Analyze {
            text: text.to_owned(),
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    /// 分析已登记声明（`POST /domains/{d}:analyze` / `pact_analyze`）。
    pub async fn analyze_loaded(&self, domain: &str) -> Result<AnalysisReport, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::AnalyzeLoaded {
            domain: domain.to_owned(),
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    /// 卸载实例（内存态；日志保留，重启自动恢复）。
    pub async fn unload_domain(&self, key: InstanceKey) -> Result<(), EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::UnloadDomain { key, resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    /// 日志前缀压缩（D25 子项 1）：写全量自足快照后，截断日志中已被覆盖的前缀。
    ///
    /// `keep_last` = 保留最近的事件条数（0 = 清空已入快照的前缀）。
    /// 语义不变量：压缩不改变实例状态、seq 续接点与 Rule-E 幂等语义；仅减少磁盘行数。
    pub async fn compact(
        &self,
        key: InstanceKey,
        keep_last: u64,
    ) -> Result<CompactionOutcome, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Compact {
            key,
            keep_last,
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn compose_and_load(
        &self,
        name: &str,
        fragments: Vec<String>,
        instance: &str,
    ) -> Result<DomainSummary, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::ComposeAndLoad {
            name: name.to_owned(),
            fragments,
            instance: instance.to_owned(),
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    /// 关停：写全部实例终态快照后退出 actor。
    pub async fn shutdown(&self) -> Result<(), EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Shutdown { resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }
}
