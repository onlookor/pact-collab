//! 引擎 actor（设计 §8）：tokio 任务 + mpsc 命令通道，独占持有全部域的
//! CounterState（D6 单权威）。每个命令处理 = 查找域 → 调用 pact-core 纯函数 →
//! 成功则 apply + 持久化 + 广播；检查与应用在同一临界区内原子完成。
//! 引擎只编排不判定（D14）：GateHolds、Rule-W、Rule-T 全部转调 core。

use crate::clock::Clock;
use crate::idgen::IdGen;
use crate::notify::{Notification, NotificationKind, NotificationSummary};
use crate::views::{AnalysisReport, ArchiveView, CounterView, DomainSummary, GateClause, StatusView};
use pact_analysis as analysis;
use pact_compose::{compose as compose_fragments, Conflict};
use pact_core::{
    enabled_productions, rule_e, rule_p, rule_r, rule_s, termination_holds, ArchiveEntry,
    CounterState, Domain, Effect, ParticipantName, ProductionOption, RuleError, ThingRef, Token,
    TokenId, TypeName,
};
use pact_parser::{emit_domain, load_domain, Problem};
use pact_store::{apply_event, Event, Snapshot, Store, StoreError};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot};

// ============ 错误 ============

#[derive(Debug)]
pub enum EngineError {
    DomainNotFound { name: String },
    Store(StoreError),
    Parse(Problem),
    Rule(RuleError),
    Compose(Vec<Conflict>),
    /// 启动恢复失败（日志/声明损坏，需人工介入；后续全部命令以此失败）。
    Recovery(String),
    /// 引擎已停止。
    EngineStopped,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::DomainNotFound { name } => write!(f, "域未加载：{name}"),
            EngineError::Store(e) => write!(f, "存储失败：{e}"),
            EngineError::Parse(e) => write!(f, "域声明无效：{e:?}"),
            EngineError::Rule(e) => write!(f, "规则拒绝：{e:?}"),
            EngineError::Compose(cs) => write!(f, "片段组合冲突：{cs:?}"),
            EngineError::Recovery(msg) => write!(f, "启动恢复失败：{msg}"),
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
    /// 每域 broadcast 通道容量（Rule-W 通知）。
    pub notify_capacity: usize,
    /// 命令通道容量。
    pub command_capacity: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self { snapshot_every: 64, notify_capacity: 256, command_capacity: 256 }
    }
}

// ============ 命令 ============

type Responder<T> = oneshot::Sender<Result<T, EngineError>>;

pub enum Command {
    LoadDomain { name: String, declaration: String, resp: Responder<DomainSummary> },
    Validate { declaration: String, resp: Responder<Domain> },
    Seed {
        domain: String,
        participant: ParticipantName,
        ty: TypeName,
        thing_ref: ThingRef,
        resp: Responder<Token>,
    },
    Produce {
        domain: String,
        participant: ParticipantName,
        ty: TypeName,
        predecessors: Vec<TokenId>,
        thing_ref: ThingRef,
        resp: Responder<Token>,
    },
    Read { domain: String, participant: ParticipantName, token_id: TokenId, resp: Responder<Token> },
    Consume {
        domain: String,
        participant: ParticipantName,
        token_id: TokenId,
        resp: Responder<ArchiveEntry>,
    },
    QueryCounter { domain: String, filter: Option<TypeName>, resp: Responder<CounterView> },
    QueryArchive { domain: String, filter: Option<TypeName>, resp: Responder<ArchiveView> },
    QueryGates { domain: String, participant: ParticipantName, resp: Responder<Vec<GateClause>> },
    QueryEnabled {
        domain: String,
        participant: ParticipantName,
        resp: Responder<Vec<ProductionOption>>,
    },
    QueryStatus { domain: String, resp: Responder<StatusView> },
    ListDomains { resp: Responder<Vec<String>> },
    /// 卸载域（内存态；日志保留，设计 §9.2 DELETE）。
    UnloadDomain { domain: String, resp: Responder<()> },
    Subscribe { domain: String, resp: Responder<broadcast::Receiver<Notification>> },
    Analyze { text: String, resp: Responder<AnalysisReport> },
    /// 分析已加载域的声明（`POST /domains/{d}:analyze`）。
    AnalyzeLoaded { domain: String, resp: Responder<AnalysisReport> },
    ComposeAndLoad { name: String, fragments: Vec<String>, resp: Responder<DomainSummary> },
    Shutdown { resp: Responder<()> },
}

impl Command {
    /// 以统一错误应答命令（启动恢复失败时排空命令队列用）。
    /// 各命令 responder 类型不同，无法合并 or-pattern，逐变体应答。
    fn fail(self, err: EngineError) {
        match self {
            Command::LoadDomain { resp, .. }
            | Command::ComposeAndLoad { resp, .. } => {
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
            Command::Shutdown { resp } => {
                let _ = resp.send(Err(err));
            }
        }
    }
}

// ============ 会话 ============

struct DomainSession {
    domain: Domain,
    state: CounterState,
    last_seq: u64,
    events_since_snapshot: u64,
    /// 当前 Rule-T 判定（信息性，不锁定域——种子仍可注入）。
    terminated: bool,
    notify: broadcast::Sender<Notification>,
}

// ============ Actor ============

struct EngineActor {
    store: Store,
    clock: Arc<dyn Clock>,
    idgen: Arc<dyn IdGen>,
    config: EngineConfig,
    domains: BTreeMap<String, DomainSession>,
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

    /// 启动恢复：store 清洗日志 + 索引校验 → 逐域重建会话。
    async fn recover(&mut self) -> Result<(), EngineError> {
        for rd in self.store.recover().await? {
            self.recover_domain(rd.name, rd.events)?;
        }
        Ok(())
    }

    /// 单域恢复：最后一个 DomainLoaded 解析声明 →
    /// 快照快速路径（snapshot.seq ≥ DomainLoaded.seq 才有效）→ 尾部重放。
    fn recover_domain(&mut self, name: String, events: Vec<Event>) -> Result<(), EngineError> {
        let Some(dl_idx) = events.iter().rposition(|e| matches!(e, Event::DomainLoaded { .. }))
        else {
            return Err(EngineError::Recovery(format!(
                "域 {name} 的日志缺少 DomainLoaded 事件（日志损坏）"
            )));
        };
        let (declaration, dl_seq) = match &events[dl_idx] {
            Event::DomainLoaded { declaration, seq, .. } => (declaration.clone(), *seq),
            _ => unreachable!(),
        };
        let domain = load_domain(&declaration).map_err(EngineError::Parse)?;

        let last_seq = events.last().map(|e| e.seq()).unwrap_or(0);
        let snap = self.store.load_snapshot(&name)?;
        let (mut state, base_seq) = match snap {
            // 快照晚于最后一次域重置才可用（否则已被重置作废）
            Some(s) if s.seq >= dl_seq && s.seq <= last_seq => (s.state, s.seq),
            _ => (CounterState::new(), dl_seq),
        };
        for ev in events.iter().filter(|e| e.seq() > base_seq) {
            apply_event(&mut state, ev)?;
        }
        let terminated = termination_holds(&domain, &state);
        let (notify, _) = broadcast::channel(self.config.notify_capacity);
        self.domains.insert(
            name,
            DomainSession {
                domain,
                state,
                last_seq,
                events_since_snapshot: 0,
                terminated,
                notify,
            },
        );
        Ok(())
    }

    /// 处理一条命令。返回 true 表示 Shutdown（退出 actor）。
    async fn handle(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::LoadDomain { name, declaration, resp } => {
                respond(resp, self.load_domain(&name, &declaration).await)
            }
            Command::Validate { declaration, resp } => {
                respond(resp, load_domain(&declaration).map_err(EngineError::Parse))
            }
            Command::Seed { domain, participant, ty, thing_ref, resp } => {
                respond(resp, self.seed(&domain, &participant, &ty, thing_ref).await)
            }
            Command::Produce { domain, participant, ty, predecessors, thing_ref, resp } => respond(
                resp,
                self.produce(&domain, &participant, &ty, &predecessors, thing_ref).await,
            ),
            Command::Read { domain, participant, token_id, resp } => {
                respond(resp, self.read(&domain, &participant, &token_id).await)
            }
            Command::Consume { domain, participant, token_id, resp } => {
                respond(resp, self.consume(&domain, &participant, &token_id).await)
            }
            Command::QueryCounter { domain, filter, resp } => {
                respond(resp, self.query_counter(&domain, filter.as_ref()))
            }
            Command::QueryArchive { domain, filter, resp } => {
                respond(resp, self.query_archive(&domain, filter.as_ref()))
            }
            Command::QueryGates { domain, participant, resp } => {
                respond(resp, self.query_gates(&domain, &participant))
            }
            Command::QueryEnabled { domain, participant, resp } => {
                respond(resp, self.query_enabled(&domain, &participant))
            }
            Command::QueryStatus { domain, resp } => respond(resp, self.query_status(&domain)),
            Command::ListDomains { resp } => respond(resp, Ok(self.domains.keys().cloned().collect())),
            Command::UnloadDomain { domain, resp } => respond(resp, self.unload_domain(&domain)),
            Command::Subscribe { domain, resp } => {
                respond(resp, self.subscribe(&domain))
            }
            Command::Analyze { text, resp } => respond(resp, self.analyze(&text)),
            Command::AnalyzeLoaded { domain, resp } => {
                respond(resp, self.analyze_loaded(&domain))
            }
            Command::ComposeAndLoad { name, fragments, resp } => {
                respond(resp, self.compose_and_load(&name, &fragments).await)
            }
            Command::Shutdown { resp } => {
                respond(resp, self.snapshot_all());
                return true;
            }
        }
        false
    }

    // —— 域生命周期 ——

    async fn load_domain(
        &mut self,
        name: &str,
        declaration: &str,
    ) -> Result<DomainSummary, EngineError> {
        pact_store::validate_domain_name(name)?;
        let domain = load_domain(declaration).map_err(EngineError::Parse)?;
        let now = self.clock.now();
        // 替换语义：同名额重置会话（快照清理 + 状态清零），seq 续接
        let next_seq = self.domains.get(name).map(|s| s.last_seq + 1).unwrap_or(1);
        let ev = Event::DomainLoaded {
            seq: next_seq,
            ts: now,
            name: name.to_owned(),
            declaration: declaration.to_owned(),
        };
        self.store.append(name, &ev).await?;
        if self.domains.contains_key(name) {
            let _ = self.store.remove_snapshot(name);
        }
        let terminated = termination_holds(&domain, &CounterState::new());
        let (notify, _) = broadcast::channel(self.config.notify_capacity);
        let summary = DomainSummary {
            name: name.to_owned(),
            types: domain.types.len(),
            gates: domain.gates.len(),
            seed_authority: domain.seed_authority.len(),
            last_seq: next_seq,
        };
        self.domains.insert(
            name.to_owned(),
            DomainSession {
                domain,
                state: CounterState::new(),
                last_seq: next_seq,
                events_since_snapshot: 0,
                terminated,
                notify,
            },
        );
        Ok(summary)
    }

    async fn compose_and_load(
        &mut self,
        name: &str,
        fragments: &[String],
    ) -> Result<DomainSummary, EngineError> {
        let mut domains = Vec::with_capacity(fragments.len());
        for f in fragments {
            domains.push(load_domain(f).map_err(EngineError::Parse)?);
        }
        let merged = compose_fragments(&domains).map_err(EngineError::Compose)?;
        let declaration = emit_domain(&merged);
        self.load_domain(name, &declaration).await
    }

    // —— 四类动作（Rule-S/P/R/E）——

    async fn seed(
        &mut self,
        name: &str,
        participant: &ParticipantName,
        ty: &TypeName,
        thing_ref: ThingRef,
    ) -> Result<Token, EngineError> {
        let id = self.idgen.next();
        let now = self.clock.now();
        // 直接字段访问：domains 的可变借用与 store/clock/config 的使用不相交
        let session = self.domains.get_mut(name).ok_or_else(|| not_found(name))?;
        let token = rule_s(&session.domain, participant, ty, thing_ref, id, now.clone())?;
        let seq = session.last_seq + 1;
        let ev = Event::Seeded { seq, ts: now.clone(), token: token.clone() };
        self.store.append(name, &ev).await?;
        session.state.apply(Effect::Seeded(token.clone()));
        session.last_seq = seq;
        notify_state_change(
            session,
            name,
            NotificationKind::Seeded,
            seq,
            &now,
            Some(token.id.clone()),
        );
        post_commit(&self.store, self.clock.as_ref(), &self.config, name, session).await?;
        Ok(token)
    }

    async fn produce(
        &mut self,
        name: &str,
        participant: &ParticipantName,
        ty: &TypeName,
        predecessors: &[TokenId],
        thing_ref: ThingRef,
    ) -> Result<Token, EngineError> {
        let id = self.idgen.next();
        let now = self.clock.now();
        let session = self.domains.get_mut(name).ok_or_else(|| not_found(name))?;
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
        let ev = Event::Produced { seq, ts: now.clone(), token: token.clone() };
        self.store.append(name, &ev).await?;
        session.state.apply(Effect::Produced(token.clone()));
        session.last_seq = seq;
        notify_state_change(
            session,
            name,
            NotificationKind::Produced,
            seq,
            &now,
            Some(token.id.clone()),
        );
        post_commit(&self.store, self.clock.as_ref(), &self.config, name, session).await?;
        Ok(token)
    }

    /// Rule-R：无状态变更、无事件（D9）、无通知。
    async fn read(
        &mut self,
        name: &str,
        participant: &ParticipantName,
        token_id: &TokenId,
    ) -> Result<Token, EngineError> {
        let session = self.session(name)?;
        rule_r(&session.domain, &session.state, participant, token_id).map_err(EngineError::Rule)
    }

    async fn consume(
        &mut self,
        name: &str,
        participant: &ParticipantName,
        token_id: &TokenId,
    ) -> Result<ArchiveEntry, EngineError> {
        let now = self.clock.now();
        let session = self.domains.get_mut(name).ok_or_else(|| not_found(name))?;
        let entry = rule_e(&session.domain, &session.state, participant, token_id, now.clone())?;
        let seq = session.last_seq + 1;
        let ev = Event::Consumed {
            seq,
            ts: now.clone(),
            token_id: token_id.clone(),
            by: participant.clone(),
        };
        self.store.append_consumed(name, &ev, &entry.token).await?;
        session.state.apply(Effect::Consumed(entry.clone()));
        session.last_seq = seq;
        notify_state_change(
            session,
            name,
            NotificationKind::Consumed,
            seq,
            &now,
            Some(token_id.clone()),
        );
        post_commit(&self.store, self.clock.as_ref(), &self.config, name, session).await?;
        Ok(entry)
    }

    // —— 查询 ——

    fn query_counter(&self, name: &str, filter: Option<&TypeName>) -> Result<CounterView, EngineError> {
        let s = self.session(name)?;
        let tokens: Vec<Token> = s
            .state
            .counter
            .values()
            .filter(|t| filter.map_or(true, |f| t.ty == *f))
            .cloned()
            .collect();
        let size = tokens.len();
        Ok(CounterView { tokens, size })
    }

    fn query_archive(&self, name: &str, filter: Option<&TypeName>) -> Result<ArchiveView, EngineError> {
        let s = self.session(name)?;
        let entries: Vec<ArchiveEntry> = s
            .state
            .archive
            .iter()
            .filter(|e| filter.map_or(true, |f| e.token.ty == *f))
            .cloned()
            .collect();
        let size = entries.len();
        Ok(ArchiveView { entries, size })
    }

    fn query_gates(&self, name: &str, participant: &ParticipantName) -> Result<Vec<GateClause>, EngineError> {
        let s = self.session(name)?;
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
        name: &str,
        participant: &ParticipantName,
    ) -> Result<Vec<ProductionOption>, EngineError> {
        let s = self.session(name)?;
        Ok(enabled_productions(&s.domain, &s.state, participant))
    }

    fn query_status(&self, name: &str) -> Result<StatusView, EngineError> {
        let s = self.session(name)?;
        Ok(StatusView {
            counter_size: s.state.counter.len(),
            archive_size: s.state.archive.len(),
            terminated: s.terminated,
        })
    }

    fn subscribe(&self, name: &str) -> Result<broadcast::Receiver<Notification>, EngineError> {
        Ok(self.session(name)?.notify.subscribe())
    }

    fn analyze(&self, text: &str) -> Result<AnalysisReport, EngineError> {
        let domain = load_domain(text).map_err(EngineError::Parse)?;
        Ok(report_of(&domain))
    }

    /// 分析已加载域（无需重新解析声明原文）。
    fn analyze_loaded(&self, name: &str) -> Result<AnalysisReport, EngineError> {
        let session = self.domains.get(name).ok_or_else(|| not_found(name))?;
        Ok(report_of(&session.domain))
    }

    /// 卸载域：仅移除内存会话；日志/快照保留，重启后自动恢复（设计 §9.2）。
    fn unload_domain(&mut self, name: &str) -> Result<(), EngineError> {
        if self.domains.remove(name).is_none() {
            return Err(not_found(name));
        }
        Ok(())
    }

    // —— 内部 ——

    fn session(&self, name: &str) -> Result<&DomainSession, EngineError> {
        self.domains.get(name).ok_or_else(|| not_found(name))
    }

    /// 关停前终态快照（全部域）。
    fn snapshot_all(&self) -> Result<(), EngineError> {
        for (name, s) in &self.domains {
            let snap = Snapshot { seq: s.last_seq, state: s.state.clone() };
            self.store.write_snapshot(name, &snap)?;
        }
        Ok(())
    }
}

/// 状态变更通知（§6.4）：变更后 Counter 规模随通知发出。
fn notify_state_change(
    session: &DomainSession,
    name: &str,
    kind: NotificationKind,
    seq: u64,
    ts: &str,
    token_id: Option<TokenId>,
) {
    let _ = session.notify.send(Notification {
        event: kind,
        domain: name.to_owned(),
        seq,
        ts: ts.to_owned(),
        token_id,
        summary: NotificationSummary { counter_size: session.state.counter.len() },
    });
}

/// 状态变更后的公共提交尾部：定期快照 + Rule-T 惰性求值。
/// 终止转移（false→true）时追加 Terminated 事件并广播；信息性，不锁定域。
async fn post_commit(
    store: &Store,
    clock: &dyn Clock,
    config: &EngineConfig,
    name: &str,
    session: &mut DomainSession,
) -> Result<(), EngineError> {
    // 定期快照（失败上报；重放兜底保证正确性不受影响，设计 §7.4）
    session.events_since_snapshot += 1;
    if config.snapshot_every > 0 && session.events_since_snapshot >= config.snapshot_every {
        let snap = Snapshot { seq: session.last_seq, state: session.state.clone() };
        store.write_snapshot(name, &snap)?;
        session.events_since_snapshot = 0;
    }

    // Rule-T 惰性求值（D14：转调 core）
    let now_terminated = termination_holds(&session.domain, &session.state);
    if now_terminated && !session.terminated {
        let seq = session.last_seq + 1;
        let ts = clock.now();
        store.append(name, &Event::Terminated { seq, ts: ts.clone() }).await?;
        session.last_seq = seq;
        let _ = session.notify.send(Notification {
            event: NotificationKind::Terminated,
            domain: name.to_owned(),
            seq,
            ts,
            token_id: None,
            summary: NotificationSummary { counter_size: session.state.counter.len() },
        });
    }
    session.terminated = now_terminated;
    Ok(())
}

fn respond<T>(resp: oneshot::Sender<Result<T, EngineError>>, result: Result<T, EngineError>) {
    let _ = resp.send(result);
}

/// 六项静态分析聚合（论文 Appendix A.1–A.6）。
fn report_of(domain: &Domain) -> AnalysisReport {
    AnalysisReport {
        reachability: analysis::reachability(domain),
        gate_coverage: analysis::gate_coverage(domain),
        consumption_reachability: analysis::consumption_reachability(domain),
        structural_deadlock: analysis::structural_deadlock(domain),
        source_production_legality: analysis::source_production_legality(domain),
        stranding_risk: analysis::stranding_risk(domain),
    }
}

fn not_found(name: &str) -> EngineError {
    EngineError::DomainNotFound { name: name.to_owned() }
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
        let actor =
            EngineActor { store, clock, idgen, config, domains: BTreeMap::new() };
        tokio::spawn(actor.run(rx));
        Self { tx }
    }

    /// 发送原始命令（接口层可直接映射）。
    pub async fn execute(&self, cmd: Command) -> Result<(), EngineError> {
        self.tx.send(cmd).await.map_err(|_| EngineError::EngineStopped)
    }

    pub async fn load_domain(
        &self,
        name: &str,
        declaration: &str,
    ) -> Result<DomainSummary, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::LoadDomain {
            name: name.to_owned(),
            declaration: declaration.to_owned(),
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn validate(&self, declaration: &str) -> Result<Domain, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Validate { declaration: declaration.to_owned(), resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn seed(
        &self,
        domain: &str,
        participant: ParticipantName,
        ty: TypeName,
        thing_ref: ThingRef,
    ) -> Result<Token, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Seed {
            domain: domain.to_owned(),
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
        domain: &str,
        participant: ParticipantName,
        ty: TypeName,
        predecessors: Vec<TokenId>,
        thing_ref: ThingRef,
    ) -> Result<Token, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Produce {
            domain: domain.to_owned(),
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
        domain: &str,
        participant: ParticipantName,
        token_id: TokenId,
    ) -> Result<Token, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Read {
            domain: domain.to_owned(),
            participant,
            token_id,
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn consume(
        &self,
        domain: &str,
        participant: ParticipantName,
        token_id: TokenId,
    ) -> Result<ArchiveEntry, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Consume {
            domain: domain.to_owned(),
            participant,
            token_id,
            resp,
        })
        .await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn counter(
        &self,
        domain: &str,
        filter: Option<TypeName>,
    ) -> Result<CounterView, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::QueryCounter { domain: domain.to_owned(), filter, resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn archive(
        &self,
        domain: &str,
        filter: Option<TypeName>,
    ) -> Result<ArchiveView, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::QueryArchive { domain: domain.to_owned(), filter, resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn my_gates(
        &self,
        domain: &str,
        participant: ParticipantName,
    ) -> Result<Vec<GateClause>, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::QueryGates { domain: domain.to_owned(), participant, resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn enabled(
        &self,
        domain: &str,
        participant: ParticipantName,
    ) -> Result<Vec<ProductionOption>, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::QueryEnabled { domain: domain.to_owned(), participant, resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn status(&self, domain: &str) -> Result<StatusView, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::QueryStatus { domain: domain.to_owned(), resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn list_domains(&self) -> Result<Vec<String>, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::ListDomains { resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn subscribe(
        &self,
        domain: &str,
    ) -> Result<broadcast::Receiver<Notification>, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Subscribe { domain: domain.to_owned(), resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn analyze(&self, text: &str) -> Result<AnalysisReport, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Analyze { text: text.to_owned(), resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    /// 分析已加载域的声明（`POST /domains/{d}:analyze` / `pact_analyze`）。
    pub async fn analyze_loaded(&self, domain: &str) -> Result<AnalysisReport, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::AnalyzeLoaded { domain: domain.to_owned(), resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    /// 卸载域（内存态；日志保留，重启自动恢复）。
    pub async fn unload_domain(&self, domain: &str) -> Result<(), EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::UnloadDomain { domain: domain.to_owned(), resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    pub async fn compose_and_load(
        &self,
        name: &str,
        fragments: Vec<String>,
    ) -> Result<DomainSummary, EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::ComposeAndLoad { name: name.to_owned(), fragments, resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }

    /// 关停：写全部域终态快照后退出 actor。
    pub async fn shutdown(&self) -> Result<(), EngineError> {
        let (resp, rx) = oneshot::channel();
        self.execute(Command::Shutdown { resp }).await?;
        rx.await.map_err(|_| EngineError::EngineStopped)?
    }
}
