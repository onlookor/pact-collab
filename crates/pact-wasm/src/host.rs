//! 宿主无关的 WASM 门面核心：声明登记、实例会话、四类动作、两阶段提交、
//! 视图查询、事件导出与恢复。
//!
//! 所有入口只接受 JSON 字符串/声明原文并返回可序列化视图；不做 IO，
//! 持久化由宿主（P4 的 JS 存储适配器）在 `prepare` 与 `commit` 之间完成。
//! 语义与原生 `pact-engine` 逐项对齐：同一 `pact-core` 规则、同一事件模型、
//! 同一 seq 分配与 Rule-T 惰性求值顺序。

use pact_compose::compose as compose_fragments;
use pact_core::{
    Domain, ParticipantName, ProductionOption, RuleError, ThingRef, Token, TokenId, TypeName,
};
use pact_engine_kernel::{analysis_report, ConsumePreparation, KernelError, Session};
use pact_parser::{load_domain, Problem};
use pact_store_model::{
    to_wire_json, validate_domain_name, validate_instance_name, Event, InstanceKey, ModelError,
    Snapshot, WireEvent, EVENT_SCHEMA_VERSION,
};
use pact_views::{
    AnalysisReport, ArchiveView, CounterView, DomainListing, DomainSummary, GateClause, StatusView,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Deref, DerefMut};

/// 默认实例名（FR19：不指定实例 ⇒ 默认实例）。
pub const DEFAULT_INSTANCE: &str = "default";

/// `load_domain`/`instantiate` 不带时间参数时的确定性占位时间戳。
pub const DEFAULT_TS: &str = "1970-01-01T00:00:00Z";

/// 门面错误：`code` 与原生接口层（REST `error.code` / `EngineError`）同名，
/// 便于三接口逐项对账；`message` 面向诊断（不含路径/凭据）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FacadeError {
    pub code: String,
    pub message: String,
}

impl FacadeError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    fn parse(problem: Problem) -> Self {
        let code = match &problem {
            Problem::Syntax(_) => "Syntax",
            Problem::WellFormedness(_) => "WellFormedness",
        };
        Self::new(code, format!("{problem:?}"))
    }

    fn model(err: ModelError) -> Self {
        let code = match &err {
            ModelError::InvalidDomainName { .. } => "InvalidDomainName",
            ModelError::InvalidInstanceName { .. } => "InvalidInstanceName",
            ModelError::UnsupportedSchemaVersion { .. } => "UnsupportedSchemaVersion",
            ModelError::Json { .. } => "ModelJson",
        };
        Self::new(code, err.to_string())
    }

    fn rule(err: RuleError) -> Self {
        let code = match &err {
            RuleError::StaticGateDenied { .. } => "StaticGateDenied",
            RuleError::GateConditionFailed { .. } => "GateConditionFailed",
            RuleError::PreconditionMismatch { .. } => "PreconditionMismatch",
            RuleError::SourceTypeProductionDenied { .. } => "SourceTypeProductionDenied",
            RuleError::PredecessorNotOnCounter { .. } => "PredecessorNotOnCounter",
            RuleError::DuplicatePredecessor { .. } => "DuplicatePredecessor",
            RuleError::TokenNotOnCounter { .. } => "TokenNotOnCounter",
            RuleError::SeedAuthorityDenied { .. } => "SeedAuthorityDenied",
        };
        Self::new(code, format!("{err:?}"))
    }

    fn kernel(err: KernelError) -> Self {
        match err {
            KernelError::Parse(p) => Self::parse(p),
            KernelError::Rule(r) => Self::rule(r),
            KernelError::Model(m) => Self::model(m),
            KernelError::Recovery(msg) => Self::new("Recovery", msg),
            KernelError::IdempotencyConflict { .. } => {
                Self::new("IdempotencyConflict", err.to_string())
            }
        }
    }
}

/// 声明校验视图（不含实例语义；`load_domain` 成功即已通过良构性校验）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidationView {
    pub types: Vec<String>,
    pub gates: usize,
    pub seed_authority: usize,
}

/// `prepare_*` 的返回：待持久化事件 + 本次命令结果。
///
/// `tx_id` / `event` 为 `None` 时表示幂等命中（`replayed = true`）：
/// 宿主不应追加任何日志行，直接采用 `result`。
#[derive(Debug, Clone, Serialize)]
pub struct Prepared {
    pub tx_id: Option<String>,
    pub seq: Option<u64>,
    /// 完整 JSONL 行（含顶层 `version`），宿主原样 append + flush。
    pub event: Option<String>,
    pub result: Value,
    pub replayed: bool,
}

/// `commit` 的返回：已确认事件行 + 可能的 Rule-T 终止事件 + 提交后状态。
#[derive(Debug, Clone, Serialize)]
pub struct Committed {
    pub tx_id: String,
    pub seq: u64,
    /// 本次命令的事件行（与 `prepare` 返回的 `event` 相同）。
    pub event: String,
    /// 提交后若首次满足 Rule-T，追加的第二条待持久化事件（原生引擎同序）。
    pub terminated: Option<TerminatedNote>,
    pub status: StatusView,
}

/// Rule-T 终止事件（信息性，无状态效果）。
#[derive(Debug, Clone, Serialize)]
pub struct TerminatedNote {
    pub seq: u64,
    pub ts: String,
    pub event: String,
}

/// 实例会话：公共内核 + 内存事件向量（供导出与跨宿主对账）。
#[derive(Debug, Clone)]
struct InstanceSession {
    core: Session,
    /// 本实例完整事件序列（含 DomainLoaded；恢复时来自宿主日志）。
    events: Vec<Event>,
}

impl Deref for InstanceSession {
    type Target = Session;

    fn deref(&self) -> &Self::Target {
        &self.core
    }
}

impl DerefMut for InstanceSession {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}

/// 待提交事务：`prepare` 时分配 seq，但不改变会话状态。
#[derive(Debug, Clone)]
struct PendingTx {
    key: InstanceKey,
    seq: u64,
    event: Event,
}

/// 内存宿主：定义键声明注册表 + 实例会话 + 待提交事务。
#[derive(Debug, Default)]
pub struct PactHost {
    /// 定义键（domain）→ 声明原文与解析结果（declare 覆盖；实例使用自己的快照）。
    declarations: BTreeMap<String, (String, Domain)>,
    /// 实例键 → 运行会话。
    sessions: BTreeMap<InstanceKey, InstanceSession>,
    /// tx_id → 待提交事件。
    pending: BTreeMap<String, PendingTx>,
    next_tx: u64,
}

impl PactHost {
    pub fn new() -> Self {
        Self::default()
    }

    // —— 域/实例生命周期 ——

    /// 以确定性占位时间戳加载域（不带 `ts` 参数时的入口）。
    pub fn load_domain(
        &mut self,
        name: &str,
        declaration: &str,
        instance: &str,
    ) -> Result<DomainSummary, FacadeError> {
        self.load_domain_at(name, declaration, instance, DEFAULT_TS, false)
    }

    /// 加载/重置实例（`force = false`，与原生 `Engine::load_domain` 同义）。
    pub fn load_domain_at(
        &mut self,
        name: &str,
        declaration: &str,
        instance: &str,
        ts: &str,
        force: bool,
    ) -> Result<DomainSummary, FacadeError> {
        let key = Self::key(name, instance)?;
        let domain = load_domain(declaration).map_err(FacadeError::parse)?;
        self.declarations
            .insert(name.to_owned(), (declaration.to_owned(), domain));

        let existing = self
            .sessions
            .get(&key)
            .map(|s| (s.is_occupied(), s.last_seq));
        match existing {
            Some((true, _)) if !force => Err(FacadeError::new(
                "DomainBusy",
                format!("实例 {key} 非空，加载将重置其 Counter；如确认替换请显式 force"),
            )),
            Some((_, last_seq)) => {
                let seq = last_seq + 1;
                self.reset_session(&key, declaration, ts, seq)
            }
            None => self.create_session(&key, declaration, ts),
        }
    }

    /// 从已登记声明创建额外实例（不改变声明）。
    pub fn instantiate(
        &mut self,
        domain: &str,
        instance: &str,
        ts: &str,
    ) -> Result<DomainSummary, FacadeError> {
        let key = Self::key(domain, instance)?;
        if self.sessions.contains_key(&key) {
            return Err(FacadeError::new(
                "InstanceExists",
                format!("实例已存在：{key}"),
            ));
        }
        let declaration = self
            .declarations
            .get(domain)
            .map(|(text, _)| text.clone())
            .ok_or_else(|| {
                FacadeError::new("DomainNotDeclared", format!("域声明未登记：{domain}"))
            })?;
        self.create_session(&key, &declaration, ts)
    }

    /// 仅校验声明（不登记、不分配实例）。
    pub fn validate(&self, declaration: &str) -> Result<ValidationView, FacadeError> {
        let domain = load_domain(declaration).map_err(FacadeError::parse)?;
        Ok(ValidationView {
            types: domain
                .types
                .iter()
                .map(|t| t.0.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            gates: domain.gates.len(),
            seed_authority: domain.seed_authority.len(),
        })
    }

    /// 六项静态分析（A.5 置于首位；与原生引擎 `report_of` 同序同字段）。
    pub fn analyze(&self, declaration: &str) -> Result<AnalysisReport, FacadeError> {
        let domain = load_domain(declaration).map_err(FacadeError::parse)?;
        Ok(analysis_report(&domain))
    }

    /// 片段并行组合（论文 §1.8）后加载为实例；`fragments_json` 为 JSON 字符串数组。
    pub fn compose(
        &mut self,
        name: &str,
        fragments_json: &str,
        instance: &str,
        ts: &str,
    ) -> Result<DomainSummary, FacadeError> {
        let fragments: Vec<String> = serde_json::from_str(fragments_json)
            .map_err(|e| FacadeError::new("BadJson", e.to_string()))?;
        let mut parsed = Vec::with_capacity(fragments.len());
        for (i, text) in fragments.iter().enumerate() {
            let domain = load_domain(text)
                .map_err(|p| FacadeError::new("Parse", format!("片段 #{i}：{p:?}")))?;
            parsed.push(domain);
        }
        let merged = compose_fragments(&parsed)
            .map_err(|cs| FacadeError::new("ComposeConflict", format!("{cs:?}")))?;
        let declaration = pact_parser::emit_domain(&merged);
        self.load_domain_at(name, &declaration, instance, ts, false)
    }

    /// 已登记实例的定义键与实例清单（排序去重）。
    pub fn list_domains(&self) -> Vec<DomainListing> {
        let mut grouped: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for key in self.sessions.keys() {
            grouped
                .entry(key.domain.clone())
                .or_default()
                .insert(key.instance.clone());
        }
        grouped
            .into_iter()
            .map(|(name, instances)| DomainListing {
                name,
                instances: instances.into_iter().collect(),
            })
            .collect()
    }

    // —— 四类动作（prepare / read）——

    pub fn prepare_seed(
        &mut self,
        key: &InstanceKey,
        participant: &str,
        ty: &str,
        thing_ref: Option<&str>,
        token_id: &str,
        ts: &str,
    ) -> Result<Prepared, FacadeError> {
        let session = self.session(key)?;
        let prepared = session
            .core
            .prepare_seed(
                &ParticipantName(participant.to_owned()),
                &TypeName(ty.to_owned()),
                thing_ref.map(ThingRef::some).unwrap_or_else(ThingRef::none),
                TokenId(token_id.to_owned()),
                ts,
            )
            .map_err(FacadeError::kernel)?;
        let seq = prepared.event.seq();
        self.enqueue(key, seq, prepared.event, &prepared.result)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_produce(
        &mut self,
        key: &InstanceKey,
        participant: &str,
        ty: &str,
        predecessors_json: &str,
        thing_ref: Option<&str>,
        token_id: &str,
        ts: &str,
    ) -> Result<Prepared, FacadeError> {
        let predecessors: Vec<TokenId> = serde_json::from_str(predecessors_json)
            .map_err(|e| FacadeError::new("BadJson", e.to_string()))?;
        let session = self.session(key)?;
        let prepared = session
            .core
            .prepare_produce(
                &ParticipantName(participant.to_owned()),
                &TypeName(ty.to_owned()),
                &predecessors,
                thing_ref.map(ThingRef::some).unwrap_or_else(ThingRef::none),
                TokenId(token_id.to_owned()),
                ts,
            )
            .map_err(FacadeError::kernel)?;
        let seq = prepared.event.seq();
        self.enqueue(key, seq, prepared.event, &prepared.result)
    }

    /// `prepare_consume`：幂等命中时不产生事件（`tx_id = None`），返回首次归档结果。
    pub fn prepare_consume(
        &mut self,
        key: &InstanceKey,
        participant: &str,
        token_id: &str,
        request_id: Option<&str>,
        ts: &str,
    ) -> Result<Prepared, FacadeError> {
        let session = self.session(key)?;
        match session
            .core
            .prepare_consume(
                &ParticipantName(participant.to_owned()),
                &TokenId(token_id.to_owned()),
                request_id,
                ts,
            )
            .map_err(FacadeError::kernel)?
        {
            ConsumePreparation::Replayed(entry) => Ok(Prepared {
                tx_id: None,
                seq: None,
                event: None,
                result: to_value(&entry)?,
                replayed: true,
            }),
            ConsumePreparation::Prepared(prepared) => {
                let prepared = *prepared;
                let seq = prepared.event.seq();
                self.enqueue(key, seq, prepared.event, &prepared.result)
            }
        }
    }

    /// Rule-R：只读校验并返回 token（无事件、无状态变更）。
    pub fn read(
        &self,
        key: &InstanceKey,
        participant: &str,
        token_id: &str,
    ) -> Result<Token, FacadeError> {
        let session = self.session(key)?;
        session
            .core
            .read(
                &ParticipantName(participant.to_owned()),
                &TokenId(token_id.to_owned()),
            )
            .map_err(FacadeError::kernel)
    }

    // —— 两阶段提交 ——

    /// 应用 `prepare` 生成的事件并确认提交。
    ///
    /// `terminated_ts` 仅用于首次触发 Rule-T 时追加的 Terminated 事件
    /// （与原生 `post_commit` 取一次新时钟一致）；`None` 时沿用命令事件时间戳。
    pub fn commit(
        &mut self,
        tx_id: &str,
        terminated_ts: Option<&str>,
    ) -> Result<Committed, FacadeError> {
        let tx = self
            .pending
            .remove(tx_id)
            .ok_or_else(|| FacadeError::new("UnknownTransaction", format!("未知事务：{tx_id}")))?;
        let event = tx.event;
        let session = self
            .sessions
            .get_mut(&tx.key)
            .ok_or_else(|| instance_not_found(&tx.key))?;

        let outcome = session.core.commit(&event).map_err(FacadeError::kernel)?;
        session.events.push(event.clone());

        // Rule-T 惰性求值（D14：转调 core），顺序与原生 post_commit 一致。
        let mut terminated = None;
        if let Some(seq) = outcome.terminated_seq {
            let ts = terminated_ts.unwrap_or(event.ts()).to_owned();
            let ev = Event::Terminated {
                seq,
                ts: ts.clone(),
            };
            session.core.record_terminated(seq);
            session.events.push(ev.clone());
            let line = to_wire_json(&ev).map_err(FacadeError::model)?;
            terminated = Some(TerminatedNote {
                seq,
                ts,
                event: line,
            });
        }

        let line = to_wire_json(&event).map_err(FacadeError::model)?;
        Ok(Committed {
            tx_id: tx_id.to_owned(),
            seq: tx.seq,
            event: line,
            terminated,
            status: outcome.status,
        })
    }

    /// 放弃事务：不改变已提交状态；若该 seq 尚是最新分配则回收，保证日志连续。
    pub fn abort(&mut self, tx_id: &str) -> Result<(), FacadeError> {
        let tx = self
            .pending
            .remove(tx_id)
            .ok_or_else(|| FacadeError::new("UnknownTransaction", format!("未知事务：{tx_id}")))?;
        if let Some(session) = self.sessions.get_mut(&tx.key) {
            session.core.abort_prepared(tx.seq);
        }
        Ok(())
    }

    // —— 查询 ——

    pub fn counter(
        &self,
        key: &InstanceKey,
        filter: Option<&str>,
    ) -> Result<CounterView, FacadeError> {
        let session = self.session(key)?;
        let filter = filter.map(|f| TypeName(f.to_owned()));
        Ok(session.core.counter(filter.as_ref()))
    }

    pub fn archive(
        &self,
        key: &InstanceKey,
        filter: Option<&str>,
    ) -> Result<ArchiveView, FacadeError> {
        let session = self.session(key)?;
        let filter = filter.map(|f| TypeName(f.to_owned()));
        Ok(session.core.archive(filter.as_ref()))
    }

    pub fn gates(
        &self,
        key: &InstanceKey,
        participant: &str,
    ) -> Result<Vec<GateClause>, FacadeError> {
        let session = self.session(key)?;
        Ok(session.core.gates(&ParticipantName(participant.to_owned())))
    }

    pub fn enabled(
        &self,
        key: &InstanceKey,
        participant: &str,
    ) -> Result<Vec<ProductionOption>, FacadeError> {
        let session = self.session(key)?;
        Ok(session
            .core
            .enabled(&ParticipantName(participant.to_owned())))
    }

    pub fn status(&self, key: &InstanceKey) -> Result<StatusView, FacadeError> {
        let session = self.session(key)?;
        Ok(session.core.status())
    }

    // —— 导出与恢复 ——

    /// 实例完整事件日志（JSONL，逐行含 `version`；行间 `\n`，末尾换行）。
    pub fn export_events(&self, key: &InstanceKey) -> Result<String, FacadeError> {
        let session = self.session(key)?;
        let mut out = String::new();
        for ev in &session.events {
            out.push_str(&to_wire_json(ev).map_err(FacadeError::model)?);
            out.push('\n');
        }
        Ok(out)
    }

    /// 当前状态快照（与原生 `Snapshot` 同构、自足）。
    pub fn export_snapshot(&self, key: &InstanceKey) -> Result<Snapshot, FacadeError> {
        let session = self.session(key)?;
        Ok(session.core.snapshot())
    }

    /// 从 JSONL 日志（可选快照）恢复实例；算法与原生 `recover_instance` 一致。
    pub fn restore(
        &mut self,
        name: &str,
        instance: &str,
        events_jsonl: &str,
        snapshot_json: Option<&str>,
    ) -> Result<StatusView, FacadeError> {
        let key = Self::key(name, instance)?;
        if self.sessions.contains_key(&key) {
            return Err(FacadeError::new(
                "InstanceExists",
                format!("实例已存在：{key}"),
            ));
        }

        let snapshot: Option<Snapshot> = match snapshot_json {
            Some(text) if !text.trim().is_empty() => {
                let snap: Snapshot = serde_json::from_str(text)
                    .map_err(|e| FacadeError::new("Recovery", e.to_string()))?;
                if snap.version > EVENT_SCHEMA_VERSION {
                    return Err(FacadeError::model(ModelError::UnsupportedSchemaVersion {
                        found: snap.version,
                        supported: EVENT_SCHEMA_VERSION,
                    }));
                }
                Some(snap)
            }
            _ => None,
        };

        let mut events = Vec::new();
        for (i, line) in events_jsonl.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let wire: WireEvent = serde_json::from_str(line).map_err(|e| {
                FacadeError::new("Recovery", format!("日志第 {} 行解析失败：{e}", i + 1))
            })?;
            events.push(wire.into_event().map_err(FacadeError::model)?);
        }

        let recovered =
            Session::recover(&events, snapshot.as_ref(), 0).map_err(FacadeError::kernel)?;
        let status = recovered.status();
        self.sessions.insert(
            key,
            InstanceSession {
                core: recovered,
                events,
            },
        );
        Ok(status)
    }

    // —— 内部 ——

    fn create_session(
        &mut self,
        key: &InstanceKey,
        declaration: &str,
        ts: &str,
    ) -> Result<DomainSummary, FacadeError> {
        self.insert_fresh(key, declaration, ts, 1)?;
        Ok(self.session(key)?.core.summary(&key.domain, &key.instance))
    }

    fn reset_session(
        &mut self,
        key: &InstanceKey,
        declaration: &str,
        ts: &str,
        seq: u64,
    ) -> Result<DomainSummary, FacadeError> {
        self.insert_fresh(key, declaration, ts, seq)?;
        Ok(self.session(key)?.core.summary(&key.domain, &key.instance))
    }

    fn insert_fresh(
        &mut self,
        key: &InstanceKey,
        declaration: &str,
        ts: &str,
        seq: u64,
    ) -> Result<(), FacadeError> {
        let fresh =
            Session::fresh(&key.domain, declaration, ts, seq).map_err(FacadeError::kernel)?;
        self.sessions.insert(
            key.clone(),
            InstanceSession {
                core: fresh.session,
                events: vec![fresh.event],
            },
        );
        Ok(())
    }

    fn enqueue(
        &mut self,
        key: &InstanceKey,
        seq: u64,
        event: Event,
        result: &impl Serialize,
    ) -> Result<Prepared, FacadeError> {
        let tx_id = format!("tx-{}", self.next_tx);
        self.next_tx += 1;
        let line = to_wire_json(&event).map_err(FacadeError::model)?;
        let result = to_value(result)?;
        if let Some(session) = self.sessions.get_mut(key) {
            session.core.mark_prepared(seq);
        }
        self.pending.insert(
            tx_id.clone(),
            PendingTx {
                key: key.clone(),
                seq,
                event,
            },
        );
        Ok(Prepared {
            tx_id: Some(tx_id),
            seq: Some(seq),
            event: Some(line),
            result,
            replayed: false,
        })
    }

    fn session(&self, key: &InstanceKey) -> Result<&InstanceSession, FacadeError> {
        self.sessions
            .get(key)
            .ok_or_else(|| instance_not_found(key))
    }

    /// 解析 `domain/instance` 形式的实例键（跨 ABI 传递用）。
    pub fn parse_key(text: &str) -> Result<InstanceKey, FacadeError> {
        let (domain, instance) = text.split_once('/').ok_or_else(|| {
            FacadeError::new(
                "InvalidInstanceKey",
                format!("实例键应为 domain/instance：{text}"),
            )
        })?;
        Self::key(domain, instance)
    }

    fn key(name: &str, instance: &str) -> Result<InstanceKey, FacadeError> {
        validate_domain_name(name).map_err(FacadeError::model)?;
        validate_instance_name(instance).map_err(FacadeError::model)?;
        Ok(InstanceKey::new(name, instance))
    }
}

fn instance_not_found(key: &InstanceKey) -> FacadeError {
    FacadeError::new("InstanceNotFound", format!("实例未加载：{key}"))
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, FacadeError> {
    serde_json::to_value(value)
        .map_err(|e| FacadeError::new("ModelJson", format!("结果序列化失败：{e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REACT_LOOP: &str = include_str!("../../../examples/domains/react_loop.pact");
    const CROSS_HARNESS: &str = include_str!("../../../examples/domains/cross_harness.pact");

    const DESIGN_PHASE: &str = r#"
types { requirement; design }
preconditions { requirement → design }
gates { architect: reads requirement, produces design }
seed_authority { external_stakeholder: seeds requirement }
"#;

    const IMPL_PHASE: &str = r#"
types { design; implementation }
preconditions { design → implementation }
gates { developer: reads design, produces implementation }
seed_authority { }
"#;

    fn key(domain: &str, instance: &str) -> InstanceKey {
        InstanceKey::new(domain, instance)
    }

    #[test]
    fn load_and_analyze_react_loop() {
        let mut host = PactHost::new();
        let s = host
            .load_domain("react_loop", REACT_LOOP, "default")
            .unwrap();
        assert_eq!(s.types, 4);
        assert_eq!(s.seed_authority, 1);
        assert_eq!(s.last_seq, 1);

        let report = host.analyze(REACT_LOOP).unwrap();
        assert!(report.precondition_wf.violations.is_empty());
        assert_eq!(report.reachability.seed_types.len(), 1);
        assert!(report.structural_deadlock.is_empty());
    }

    #[test]
    fn reload_resets_empty_instance_but_refuses_occupied() {
        let mut host = PactHost::new();
        host.load_domain("cross", CROSS_HARNESS, "default").unwrap();
        // 空实例（仅 DomainLoaded）：同原生引擎，允许重置，seq 续接。
        let s = host.load_domain("cross", CROSS_HARNESS, "default").unwrap();
        assert_eq!(s.last_seq, 2);

        // 注入种子后实例非空：未 force 加载 → DomainBusy。
        let k = key("cross", "default");
        let p = host
            .prepare_seed(&k, "user", "task_brief", None, "seq-0", "T1")
            .unwrap();
        host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();
        let err = host
            .load_domain("cross", CROSS_HARNESS, "default")
            .unwrap_err();
        assert_eq!(err.code, "DomainBusy");

        // 同一声明可承载第二个实例（D27 双键）。
        host.load_domain("cross", CROSS_HARNESS, "second").unwrap();
        let listing = host.list_domains();
        assert_eq!(listing.len(), 1);
        assert_eq!(listing[0].instances, vec!["default", "second"]);
    }

    #[test]
    fn invalid_key_is_rejected_before_parse() {
        let mut host = PactHost::new();
        let err = host
            .load_domain("../etc", "types { a }", "default")
            .unwrap_err();
        assert_eq!(err.code, "InvalidDomainName");
    }

    #[test]
    fn compose_fragments_merges_shared_type() {
        let mut host = PactHost::new();
        let fragments = serde_json::to_string(&[DESIGN_PHASE, IMPL_PHASE]).unwrap();
        let s = host
            .compose("pipeline", &fragments, "default", DEFAULT_TS)
            .unwrap();
        assert_eq!(s.types, 3);
    }

    #[test]
    fn validate_reports_counts() {
        let host = PactHost::new();
        let v = host.validate(DESIGN_PHASE).unwrap();
        assert_eq!(v.types, vec!["design", "requirement"]);
        // gates 按 (participant, action, type) 计：reads requirement + produces design
        assert_eq!(v.gates, 2);
        assert_eq!(v.seed_authority, 1);
    }

    #[test]
    fn abort_keeps_state_and_reclaims_seq() {
        let mut host = PactHost::new();
        host.load_domain("d", DESIGN_PHASE, "default").unwrap();
        let k = key("d", "default");
        let p = host
            .prepare_seed(
                &k,
                "external_stakeholder",
                "requirement",
                None,
                "seq-0",
                "T1",
            )
            .unwrap();
        let seq = p.seq.unwrap();
        host.abort(p.tx_id.as_deref().unwrap()).unwrap();
        assert_eq!(host.status(&k).unwrap().counter_size, 0);
        // 回收后下一次 prepare 复用同一 seq，日志保持连续。
        let p2 = host
            .prepare_seed(
                &k,
                "external_stakeholder",
                "requirement",
                None,
                "seq-0",
                "T1",
            )
            .unwrap();
        assert_eq!(p2.seq.unwrap(), seq);
    }

    #[test]
    fn idempotent_consume_replays_without_event() {
        // 该用例需要 architect 同时具备 consume 授权。
        const CONSUME_DOMAIN: &str = r#"
types { requirement; design }
preconditions { requirement → design }
gates { architect: reads requirement, consumes requirement, produces design }
seed_authority { external_stakeholder: seeds requirement }
"#;
        let mut host = PactHost::new();
        host.load_domain("d", CONSUME_DOMAIN, "default").unwrap();
        let k = key("d", "default");
        let seed = host
            .prepare_seed(
                &k,
                "external_stakeholder",
                "requirement",
                None,
                "seq-0",
                "T1",
            )
            .unwrap();
        host.commit(seed.tx_id.as_deref().unwrap(), None).unwrap();
        let first = host
            .prepare_consume(&k, "architect", "seq-0", Some("rid-1"), "T2")
            .unwrap();
        host.commit(first.tx_id.as_deref().unwrap(), None).unwrap();
        let again = host
            .prepare_consume(&k, "architect", "seq-0", Some("rid-1"), "T3")
            .unwrap();
        assert!(again.replayed);
        assert!(again.tx_id.is_none());
        assert_eq!(again.result, first.result);

        let err = host
            .prepare_consume(&k, "architect", "seq-9", Some("rid-1"), "T4")
            .unwrap_err();
        assert_eq!(err.code, "IdempotencyConflict");
    }

    #[test]
    fn export_restore_roundtrip() {
        let mut host = PactHost::new();
        host.load_domain("d", DESIGN_PHASE, "default").unwrap();
        let k = key("d", "default");
        let seed = host
            .prepare_seed(
                &k,
                "external_stakeholder",
                "requirement",
                None,
                "seq-0",
                "T1",
            )
            .unwrap();
        host.commit(seed.tx_id.as_deref().unwrap(), None).unwrap();
        let lines = host.export_events(&k).unwrap();
        let snap = serde_json::to_string(&host.export_snapshot(&k).unwrap()).unwrap();

        let mut restored = PactHost::new();
        let status = restored
            .restore("d", "default", &lines, Some(&snap))
            .unwrap();
        assert_eq!(status, host.status(&k).unwrap());
        assert_eq!(
            restored.counter(&k, None).unwrap(),
            host.counter(&k, None).unwrap()
        );
        assert_eq!(restored.export_events(&k).unwrap(), lines);
    }
}
