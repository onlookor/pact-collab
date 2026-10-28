//! 引擎 actor（设计 §8）：tokio 任务 + mpsc 命令通道，独占持有全部域的
//! CounterState（D6 单权威）。每个命令处理 = 查找域 → 调用 pact-core 纯函数 →
//! 成功则 apply + 持久化 + 广播；检查与应用在同一临界区内原子完成。
//! 引擎只编排不判定（D14）：GateHolds、Rule-W、Rule-T 全部转调 core。

use crate::clock::Clock;
use crate::idgen::IdGen;
use crate::notify::{Notification, NotificationKind, NotificationSummary};
use crate::views::{
    AnalysisReport, ArchiveView, CompactionOutcome, CounterView, DomainListing, DomainSummary,
    GateClause, HandoffOutcome, StatusView,
};
use pact_compose::{compose as compose_fragments, Conflict};
use pact_core::{
    ArchiveEntry, Domain, ParticipantName, ProductionOption, RuleError, ThingRef, Token, TokenId,
    TypeName,
};
use pact_engine_kernel::{analysis_report, ConsumePreparation, KernelError, Session};
use pact_parser::{emit_domain, load_domain, Problem};
use pact_store::{
    ActivePointer, Event, InstanceKey, ModelError, ReadAuditRecord, Snapshot, Store, StoreError,
    ViewAuditType, ViewName,
};
use std::collections::{BTreeMap, BTreeSet};
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
    /// 实例已冻结（状态交接后的源实例只读）。
    InstanceFrozen {
        domain: String,
        instance: String,
    },
    /// 状态交接不兼容（如源 Counter 中仍有新声明未声明的类型）。
    HandoffIncompatible {
        detail: String,
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
            EngineError::InstanceFrozen { domain, instance } => {
                write!(f, "实例 {domain}/{instance} 已冻结（状态交接后只读）")
            }
            EngineError::HandoffIncompatible { detail } => {
                write!(f, "状态交接不兼容：{detail}")
            }
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

impl From<ModelError> for EngineError {
    fn from(e: ModelError) -> Self {
        EngineError::Store(e.into())
    }
}

impl From<RuleError> for EngineError {
    fn from(e: RuleError) -> Self {
        EngineError::Rule(e)
    }
}

impl From<KernelError> for EngineError {
    fn from(e: KernelError) -> Self {
        match e {
            KernelError::Parse(p) => EngineError::Parse(p),
            KernelError::Rule(r) => EngineError::Rule(r),
            KernelError::Model(m) => EngineError::Store(m.into()),
            KernelError::Recovery(msg) => EngineError::Recovery(msg),
            KernelError::IdempotencyConflict {
                request_id,
                existing,
                requested,
            } => EngineError::IdempotencyConflict {
                request_id,
                existing,
                requested,
            },
        }
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
    /// 是否持久化 Rule-R 读审计（D25 子项 3；默认关闭）。
    /// 开启后写审计失败即拒绝本次读取（fail-closed）。
    pub read_audit: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            snapshot_every: 64,
            notify_capacity: 256,
            command_capacity: 256,
            compact_on_start: false,
            compact_keep_last: 0,
            read_audit: false,
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
    /// 读审计视图（D25 子项 3 / D43）：最近 `limit` 条记录（时间正序）。
    QueryReadAudit {
        key: InstanceKey,
        limit: usize,
        resp: Responder<Vec<ReadAuditRecord>>,
    },
    /// 追加视图级读审计（D43）：`read_audit = false` 时为空操作。
    RecordViewAudit {
        key: InstanceKey,
        participant: Option<String>,
        view: ViewName,
        type_filter: Option<String>,
        types: Vec<ViewAuditType>,
        resp: Responder<()>,
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
    /// 显式状态交接：冻结源实例 → 以新声明创建目标实例并继承状态 → 可选切别名。
    Handoff {
        domain: String,
        source_instance: String,
        target_instance: String,
        declaration: String,
        activate: bool,
        /// 控制面审计：发起者（信任声明模式下为 `None`）。
        actor: Option<String>,
        resp: Responder<HandoffOutcome>,
    },
    /// 设置实例冻结状态（只读护栏；冻结标记持久化）。
    SetFrozen {
        key: InstanceKey,
        frozen: bool,
        actor: Option<String>,
        resp: Responder<()>,
    },
    /// 切换域的活动实例指针（短路由别名）。
    Activate {
        domain: String,
        instance: String,
        actor: Option<String>,
        resp: Responder<String>,
    },
    /// 查询域的活动实例（无指针 → `default`）。
    ActiveInstance {
        domain: String,
        resp: Responder<String>,
    },
    /// 查询域的活动实例指针（含审计字段；无指针 → `None`）。
    ActivePointer {
        domain: String,
        resp: Responder<Option<ActivePointer>>,
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
            Command::QueryReadAudit { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::RecordViewAudit { resp, .. } => {
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
            Command::Handoff { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::SetFrozen { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::Activate { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::ActiveInstance { resp, .. } => {
                let _ = resp.send(Err(err));
            }
            Command::ActivePointer { resp, .. } => {
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

/// 实例会话（**实例键** `InstanceKey`）：公共内核 + 原生运行态
/// （事件追加计数用于快照频率、广播通道用于 Rule-W）。
struct InstanceSession {
    /// 与 WASM 宿主共用的纯内核（声明快照、状态、seq、幂等表、终止判定）。
    core: Session,
    events_since_snapshot: u64,
    notify: broadcast::Sender<Notification>,
    /// 实例冻结：只读护栏（Seed/Produce/Consume 拒绝），持久化在 `.sealed` 标记。
    frozen: bool,
}

impl std::ops::Deref for InstanceSession {
    type Target = Session;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl std::ops::DerefMut for InstanceSession {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
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

        // 活动实例指针校验：悬空指针会让短路由静默指向不存在的实例，
        // 属数据/运维损坏，fail-fast 要求人工修正。
        let domains: BTreeSet<String> = self.instances.keys().map(|k| k.domain.clone()).collect();
        for domain in domains {
            if let Some(instance) = self.store.load_active(&domain)? {
                let key = InstanceKey::new(&domain, &instance);
                if !self.instances.contains_key(&key) {
                    return Err(EngineError::Recovery(format!(
                        "活动实例指针悬空：{domain}/active.json → {instance}（实例不存在）"
                    )));
                }
            }
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

    /// 单实例恢复：委托 `pact-engine-kernel::Session::recover`（日志最后一个
    /// `DomainLoaded` 优先；前缀被压缩时以快照为锚，再重放尾部事件）。
    fn recover_instance(&mut self, ri: pact_store::RecoveredInstance) -> Result<(), EngineError> {
        let key = ri.key;
        let snap = self.store.load_snapshot(&key)?;
        let core = Session::recover(&ri.events, snap.as_ref(), ri.base_seq)?;
        let frozen = self.store.is_sealed(&key);
        let (notify, _) = broadcast::channel(self.config.notify_capacity);
        self.instances.insert(
            key,
            InstanceSession {
                core,
                events_since_snapshot: 0,
                notify,
                frozen,
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
            Command::QueryReadAudit { key, limit, resp } => {
                respond(resp, self.query_read_audit(&key, limit))
            }
            Command::RecordViewAudit {
                key,
                participant,
                view,
                type_filter,
                types,
                resp,
            } => respond(
                resp,
                self.record_view_audit(&key, participant, view, type_filter, types),
            ),
            Command::ListDomains { resp } => respond(resp, self.list_domains()),
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
            Command::Handoff {
                domain,
                source_instance,
                target_instance,
                declaration,
                activate,
                actor,
                resp,
            } => respond(
                resp,
                self.handoff(
                    &domain,
                    &source_instance,
                    &target_instance,
                    &declaration,
                    activate,
                    actor.as_deref(),
                )
                .await,
            ),
            Command::SetFrozen {
                key,
                frozen,
                actor,
                resp,
            } => respond(resp, self.set_frozen(&key, frozen, actor.as_deref())),
            Command::Activate {
                domain,
                instance,
                actor,
                resp,
            } => respond(resp, self.activate(&domain, &instance, actor.as_deref())),
            Command::ActiveInstance { domain, resp } => {
                respond(resp, self.active_instance(&domain))
            }
            Command::ActivePointer { domain, resp } => respond(resp, self.active_pointer(&domain)),
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
    fn list_domains(&self) -> Result<Vec<DomainListing>, EngineError> {
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
        let mut out = Vec::with_capacity(listings.len());
        for (name, mut instances) in listings {
            instances.sort();
            instances.dedup();
            let active = self.store.load_active(&name)?;
            out.push(DomainListing {
                name,
                instances,
                active,
            });
        }
        Ok(out)
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
        // 冻结标记优先于“新建”：卸载后的冻结实例不能借实例化绕过冻结。
        // 显式解冻（或带 force 的重载）清标记后才可重建。
        if self.store.is_sealed(&key) {
            return Err(EngineError::InstanceFrozen {
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
        let next_seq = self.store.last_seq(&key) + 1;
        let fresh = Session::fresh(name, &declaration, &self.clock.now(), next_seq)?;
        self.store.append(&key, &fresh.event).await?;
        let summary = fresh.session.summary(name, instance);
        let (notify, _) = broadcast::channel(self.config.notify_capacity);
        self.instances.insert(
            key,
            InstanceSession {
                core: fresh.session,
                events_since_snapshot: 0,
                notify,
                frozen: false,
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
            // 冻结实例的重载等同重置：须显式 force 才解除冻结。
            Some(_) if self.store.is_sealed(&key) && !force => Err(EngineError::InstanceFrozen {
                domain: name.to_owned(),
                instance: instance.to_owned(),
            }),
            // 已存在（空或已 force）：重置语义 —— 清快照、状态清零、seq 续接
            Some(s) => {
                let next_seq = s.last_seq + 1;
                let fresh = Session::fresh(name, declaration, &self.clock.now(), next_seq)?;
                self.store.append(&key, &fresh.event).await?;
                let _ = self.store.remove_snapshot(&key);
                // 显式 force 重置即解除交接冻结。
                let _ = self.store.clear_sealed(&key);
                let summary = fresh.session.summary(name, instance);
                let (notify, _) = broadcast::channel(self.config.notify_capacity);
                self.instances.insert(
                    key,
                    InstanceSession {
                        core: fresh.session,
                        events_since_snapshot: 0,
                        notify,
                        frozen: false,
                    },
                );
                Ok(summary)
            }
            // 新实例：冻结标记只能由显式 force 清除。
            None => {
                if self.store.is_sealed(&key) {
                    if !force {
                        return Err(EngineError::InstanceFrozen {
                            domain: name.to_owned(),
                            instance: instance.to_owned(),
                        });
                    }
                    self.store.clear_sealed(&key)?;
                }
                self.instantiate(name, instance).await
            }
        }
    }

    // —— 状态交接 + 活动实例别名 ——

    /// 冻结源实例 → 以新声明创建目标实例并继承 Counter/Archive/幂等表 →
    /// 可选切换活动实例指针。目标日志以自足的 `Imported` 事件为基线。
    async fn handoff(
        &mut self,
        domain: &str,
        source_instance: &str,
        target_instance: &str,
        declaration: &str,
        activate: bool,
        actor: Option<&str>,
    ) -> Result<HandoffOutcome, EngineError> {
        pact_store::validate_domain_name(domain)?;
        pact_store::validate_instance_name(source_instance)?;
        pact_store::validate_instance_name(target_instance)?;
        if source_instance == target_instance {
            return Err(EngineError::HandoffIncompatible {
                detail: "源与目标实例不能相同".to_owned(),
            });
        }

        let source_key = InstanceKey::new(domain, source_instance);
        let target_key = InstanceKey::new(domain, target_instance);
        let new_domain = load_domain(declaration).map_err(EngineError::Parse)?;
        // 控制面审计时间：冻结标记、导入事件、活动指针共用同一时刻。
        let ts = self.clock.now();

        // 目标必须不存在（内存、日志或残留冻结标记），避免覆盖已有审计。
        if self.instances.contains_key(&target_key)
            || self.store.list_instances().iter().any(|k| k == &target_key)
            || self.store.is_sealed(&target_key)
        {
            return Err(EngineError::InstanceExists {
                domain: domain.to_owned(),
                instance: target_instance.to_owned(),
            });
        }

        let (source_seq, source_state, source_requests) = {
            let session = self
                .instances
                .get(&source_key)
                .ok_or_else(|| instance_not_found(&source_key))?;
            if session.frozen {
                return Err(EngineError::InstanceFrozen {
                    domain: domain.to_owned(),
                    instance: source_instance.to_owned(),
                });
            }
            (
                session.last_seq,
                session.state.clone(),
                session.consumed_requests.clone(),
            )
        };

        // 兼容性下界：Counter 中所有在途 token 的类型必须仍被新声明声明。
        // Archive 允许保留已删除类型（历史只读，不影响未来规则）。
        let missing: std::collections::BTreeSet<&TypeName> = source_state
            .counter
            .values()
            .map(|t| &t.ty)
            .filter(|ty| !new_domain.types.contains(*ty))
            .collect();
        if !missing.is_empty() {
            return Err(EngineError::HandoffIncompatible {
                detail: format!(
                    "源 Counter 中存在新声明未声明的类型：{}",
                    missing
                        .iter()
                        .map(|t| t.0.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }

        // 先冻结源（持久化标记）；失败则不产生任何交接副作用。
        {
            let session = self
                .instances
                .get_mut(&source_key)
                .ok_or_else(|| instance_not_found(&source_key))?;
            self.store
                .write_sealed(&source_key, "state handoff source", &ts, actor)?;
            session.frozen = true;
        }

        let counter_len = source_state.counter.len();
        let archive_len = source_state.archive.len();
        let idem_len = source_requests.len();
        let target_seq = self.store.last_seq(&target_key) + 1;
        let imported = Event::Imported {
            seq: target_seq,
            ts: ts.clone(),
            name: domain.to_owned(),
            declaration: declaration.to_owned(),
            source_domain: domain.to_owned(),
            source_instance: source_instance.to_owned(),
            source_seq,
            state: source_state,
            consumed_requests: source_requests,
        };
        if let Err(e) = self.store.append(&target_key, &imported).await {
            // 提交点失败：回滚冻结，源实例恢复可写。
            let _ = self.store.clear_sealed(&source_key);
            if let Some(s) = self.instances.get_mut(&source_key) {
                s.frozen = false;
            }
            return Err(e.into());
        }

        let core = Session::recover(std::slice::from_ref(&imported), None, target_seq)?;
        let (notify, _) = broadcast::channel(self.config.notify_capacity);
        self.instances.insert(
            target_key,
            InstanceSession {
                core,
                events_since_snapshot: 0,
                notify,
                frozen: false,
            },
        );

        // 新声明登记为域定义，供后续新实例使用（目标实例已绑定自己的快照）。
        self.declare(domain, declaration)?;

        let mut activated = false;
        if activate {
            match self.store.write_active(domain, target_instance, &ts, actor) {
                Ok(()) => activated = true,
                Err(e) => eprintln!("pactd: 交接后写活动实例指针失败（域 {domain}）：{e}"),
            }
        }
        let active_instance = self.active_instance(domain)?;

        Ok(HandoffOutcome {
            domain: domain.to_owned(),
            source_instance: source_instance.to_owned(),
            target_instance: target_instance.to_owned(),
            source_seq,
            target_seq,
            counter: counter_len,
            archive: archive_len,
            consumed_requests: idem_len,
            active_instance,
            activated,
        })
    }

    /// 设置/解除实例冻结（只读护栏）。
    fn set_frozen(
        &mut self,
        key: &InstanceKey,
        frozen: bool,
        actor: Option<&str>,
    ) -> Result<(), EngineError> {
        let session = self
            .instances
            .get_mut(key)
            .ok_or_else(|| instance_not_found(key))?;
        if frozen {
            let ts = self.clock.now();
            self.store.write_sealed(key, "manual freeze", &ts, actor)?;
        } else {
            self.store.clear_sealed(key)?;
        }
        session.frozen = frozen;
        Ok(())
    }

    /// 切换域的活动实例指针（短路由别名）。
    fn activate(
        &mut self,
        domain: &str,
        instance: &str,
        actor: Option<&str>,
    ) -> Result<String, EngineError> {
        pact_store::validate_domain_name(domain)?;
        pact_store::validate_instance_name(instance)?;
        let key = InstanceKey::new(domain, instance);
        if !self.instances.contains_key(&key) {
            return Err(instance_not_found(&key));
        }
        let ts = self.clock.now();
        self.store.write_active(domain, instance, &ts, actor)?;
        Ok(instance.to_owned())
    }

    /// 查询域的活动实例（无指针 → `default`）。
    fn active_instance(&self, domain: &str) -> Result<String, EngineError> {
        Ok(self
            .store
            .load_active(domain)?
            .unwrap_or_else(|| DEFAULT_INSTANCE.to_owned()))
    }

    /// 查询域的活动实例指针（含审计字段；无指针 → `None`）。
    fn active_pointer(&self, domain: &str) -> Result<Option<ActivePointer>, EngineError> {
        Ok(self.store.load_active_pointer(domain)?)
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
        if session.frozen {
            return Err(instance_frozen(key));
        }
        let prepared = session
            .core
            .prepare_seed(participant, ty, thing_ref, id, &now)?;
        let seq = prepared.event.seq();
        let token = prepared.result.clone();
        self.store.append(key, &prepared.event).await?;
        let outcome = session.core.commit(&prepared.event)?;
        notify_state_change(
            session,
            key,
            NotificationKind::Seeded,
            seq,
            &now,
            Some(token.id.clone()),
        );
        post_commit(
            &self.store,
            self.clock.as_ref(),
            &self.config,
            key,
            session,
            outcome.terminated_seq,
        )
        .await;
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
        if session.frozen {
            return Err(instance_frozen(key));
        }
        let prepared =
            session
                .core
                .prepare_produce(participant, ty, predecessors, thing_ref, id, &now)?;
        let seq = prepared.event.seq();
        let token = prepared.result.clone();
        self.store.append(key, &prepared.event).await?;
        let outcome = session.core.commit(&prepared.event)?;
        notify_state_change(
            session,
            key,
            NotificationKind::Produced,
            seq,
            &now,
            Some(token.id.clone()),
        );
        post_commit(
            &self.store,
            self.clock.as_ref(),
            &self.config,
            key,
            session,
            outcome.terminated_seq,
        )
        .await;
        Ok(token)
    }

    /// Rule-R：无状态变更、无事件（D9）、无通知。
    /// D25 子项 3：开启读审计时另写独立审计流；写失败即拒绝本次读取（fail-closed）。
    async fn read(
        &mut self,
        key: &InstanceKey,
        participant: &ParticipantName,
        token_id: &TokenId,
    ) -> Result<Token, EngineError> {
        let token = {
            let session = self.session(key)?;
            session.core.read(participant, token_id)?
        };
        if self.config.read_audit {
            let rec = ReadAuditRecord::rule_r(
                self.clock.now(),
                key.domain.clone(),
                key.instance.clone(),
                participant.0.clone(),
                token.id.0.clone(),
            );
            self.store.append_read_audit(key, &rec)?;
        }
        Ok(token)
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
        let prepared = match session.core.prepare_consume(
            participant,
            token_id,
            request_id.as_deref(),
            &now,
        )? {
            ConsumePreparation::Replayed(entry) => return Ok(entry),
            ConsumePreparation::Prepared(prepared) => {
                // 冻结只拦截新消费；幂等重放仍返回首次归档结果（无状态变更）。
                if session.frozen {
                    return Err(instance_frozen(key));
                }
                *prepared
            }
        };
        let seq = prepared.event.seq();
        // 提交点 = 日志落盘（store.append 内部 fsync）。其后步骤均不可失败（见 post_commit）。
        self.store.append(key, &prepared.event).await?;
        let outcome = session.core.commit(&prepared.event)?;
        notify_state_change(
            session,
            key,
            NotificationKind::Consumed,
            seq,
            &now,
            Some(token_id.clone()),
        );
        post_commit(
            &self.store,
            self.clock.as_ref(),
            &self.config,
            key,
            session,
            outcome.terminated_seq,
        )
        .await;
        Ok(prepared.result)
    }

    // —— 查询 ——

    fn query_counter(
        &self,
        key: &InstanceKey,
        filter: Option<&TypeName>,
    ) -> Result<CounterView, EngineError> {
        Ok(self.session(key)?.core.counter(filter))
    }

    fn query_archive(
        &self,
        key: &InstanceKey,
        filter: Option<&TypeName>,
    ) -> Result<ArchiveView, EngineError> {
        Ok(self.session(key)?.core.archive(filter))
    }

    fn query_gates(
        &self,
        key: &InstanceKey,
        participant: &ParticipantName,
    ) -> Result<Vec<GateClause>, EngineError> {
        Ok(self.session(key)?.core.gates(participant))
    }

    fn query_enabled(
        &self,
        key: &InstanceKey,
        participant: &ParticipantName,
    ) -> Result<Vec<ProductionOption>, EngineError> {
        Ok(self.session(key)?.core.enabled(participant))
    }

    fn query_status(&self, key: &InstanceKey) -> Result<StatusView, EngineError> {
        Ok(self.session(key)?.core.status())
    }

    /// 读审计视图（D25 子项 3 / D43）：独立审计流，实例未加载也可查看历史记录。
    fn query_read_audit(
        &self,
        key: &InstanceKey,
        limit: usize,
    ) -> Result<Vec<ReadAuditRecord>, EngineError> {
        Ok(self.store.read_audit_tail(key, limit)?)
    }

    /// 追加视图级读审计（D43）。由引擎 actor 单写者落盘；`read_audit = false` 时无操作。
    /// 开启时写失败上抛 `EngineError::Store`，接口层据此拒绝本次视图响应（fail-closed）。
    fn record_view_audit(
        &self,
        key: &InstanceKey,
        participant: Option<String>,
        view: ViewName,
        type_filter: Option<String>,
        types: Vec<ViewAuditType>,
    ) -> Result<(), EngineError> {
        if !self.config.read_audit {
            return Ok(());
        }
        let rec = ReadAuditRecord::view(
            self.clock.now(),
            key.domain.clone(),
            key.instance.clone(),
            participant,
            view,
            type_filter,
            types,
        );
        self.store.append_read_audit(key, &rec)?;
        Ok(())
    }

    fn subscribe(
        &self,
        key: &InstanceKey,
    ) -> Result<broadcast::Receiver<Notification>, EngineError> {
        Ok(self.session(key)?.notify.subscribe())
    }

    fn analyze(&self, text: &str) -> Result<AnalysisReport, EngineError> {
        let domain = load_domain(text).map_err(EngineError::Parse)?;
        Ok(analysis_report(&domain))
    }

    /// 分析已登记声明（定义级，无需实例）。
    fn analyze_loaded(&self, name: &str) -> Result<AnalysisReport, EngineError> {
        let entry = self
            .declarations
            .get(name)
            .ok_or_else(|| domain_not_declared(name))?;
        Ok(analysis_report(&entry.domain))
    }

    /// 卸载实例：仅移除内存会话；日志/快照保留，重启后自动恢复（设计 §9.2）。
    fn unload_domain(&mut self, key: &InstanceKey) -> Result<(), EngineError> {
        if self.instances.remove(key).is_none() {
            return Err(instance_not_found(key));
        }
        // 卸载的活动实例不能留下悬空别名：清除指针，短路由回退 `default`。
        if self.store.load_active(&key.domain)?.as_deref() == Some(key.instance.as_str()) {
            self.store.clear_active(&key.domain)?;
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
    terminated_seq: Option<u64>,
) {
    // 定期快照（尽力而为）
    session.events_since_snapshot += 1;
    if config.snapshot_every > 0
        && session.events_since_snapshot >= config.snapshot_every
        && store.write_snapshot(key, &snapshot_of(session)).is_ok()
    {
        session.events_since_snapshot = 0;
    }

    // Rule-T 惰性求值：候选由内核 `commit` 给出（D14）
    if let Some(seq) = terminated_seq {
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
            session.core.record_terminated(seq);
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
}

/// 实例最后一次 `DomainLoaded` 的声明原文（恢复回填用）。
fn last_declaration(events: &[Event]) -> Option<String> {
    events
        .iter()
        .rev()
        .find_map(|e| e.baseline_declaration().map(|(_, d)| d.to_owned()))
}

fn respond<T>(resp: oneshot::Sender<Result<T, EngineError>>, result: Result<T, EngineError>) {
    let _ = resp.send(result);
}

fn instance_not_found(key: &InstanceKey) -> EngineError {
    EngineError::InstanceNotFound {
        domain: key.domain.clone(),
        instance: key.instance.clone(),
    }
}

fn instance_frozen(key: &InstanceKey) -> EngineError {
    EngineError::InstanceFrozen {
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

    /// 状态交接：冻结源、以新声明创建目标并继承状态，可选切换活动实例。
    pub async fn handoff(
        &self,
        domain: &str,
        source_instance: &str,
        target_instance: &str,
        declaration: &str,
        activate: bool,
    ) -> Result<HandoffOutcome, EngineError> {
        self.handoff_as(
            domain,
            source_instance,
            target_instance,
            declaration,
            activate,
            None,
        )
        .await
    }

    /// 状态交接（带控制面审计操作者）。
    pub async fn handoff_as(
        &self,
        domain: &str,
        source_instance: &str,
        target_instance: &str,
        declaration: &str,
        activate: bool,
        actor: Option<&str>,
    ) -> Result<HandoffOutcome, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Handoff {
            domain: domain.to_owned(),
            source_instance: source_instance.to_owned(),
            target_instance: target_instance.to_owned(),
            declaration: declaration.to_owned(),
            activate,
            actor: actor.map(str::to_owned),
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    /// 设置/解除实例冻结。
    pub async fn set_frozen(&self, key: InstanceKey, frozen: bool) -> Result<(), EngineError> {
        self.set_frozen_as(key, frozen, None).await
    }

    /// 设置/解除实例冻结（带控制面审计操作者）。
    pub async fn set_frozen_as(
        &self,
        key: InstanceKey,
        frozen: bool,
        actor: Option<&str>,
    ) -> Result<(), EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::SetFrozen {
            key,
            frozen,
            actor: actor.map(str::to_owned),
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    /// 切换域的活动实例指针，返回新的活动实例名。
    pub async fn activate(&self, domain: &str, instance: &str) -> Result<String, EngineError> {
        self.activate_as(domain, instance, None).await
    }

    /// 切换域的活动实例指针（带控制面审计操作者）。
    pub async fn activate_as(
        &self,
        domain: &str,
        instance: &str,
        actor: Option<&str>,
    ) -> Result<String, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Activate {
            domain: domain.to_owned(),
            instance: instance.to_owned(),
            actor: actor.map(str::to_owned),
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    /// 查询域的活动实例（无指针 → `default`）。
    pub async fn active_instance(&self, domain: &str) -> Result<String, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::ActiveInstance {
            domain: domain.to_owned(),
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    /// 查询域的活动实例指针（含 `ts`/`by` 审计字段；无指针 → `None`）。
    pub async fn active_pointer(&self, domain: &str) -> Result<Option<ActivePointer>, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::ActivePointer {
            domain: domain.to_owned(),
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

    /// 读审计视图（D25 子项 3 / D43）：最近 `limit` 条记录（时间正序）。
    pub async fn read_audit(
        &self,
        key: InstanceKey,
        limit: usize,
    ) -> Result<Vec<ReadAuditRecord>, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::QueryReadAudit { key, limit, resp })
            .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    /// 追加视图级读审计（D43）。`read_audit = false` 时为空操作；
    /// 开启时写失败返回 `EngineError::Store`，调用方必须拒绝本次视图响应（fail-closed）。
    pub async fn record_view_audit(
        &self,
        key: InstanceKey,
        participant: Option<String>,
        view: ViewName,
        type_filter: Option<String>,
        types: Vec<ViewAuditType>,
    ) -> Result<(), EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::RecordViewAudit {
            key,
            participant,
            view,
            type_filter,
            types,
            resp,
        })
        .await?;
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
