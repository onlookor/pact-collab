//! PACT 纯内核：原生 actor 与 WASM 宿主共用的实例会话语义。
//!
//! 本 crate **无 IO、无异步、无第三方运行时**：只做
//! - 声明解析与实例创建/重置（`DomainLoaded` 事件）
//! - Rule-S/P/R/E 的纯校验与事件生成（`prepare_*` / `read`）
//! - 事件提交后的状态应用、Rule-E 幂等表与 Rule-T 终止判定
//! - 从「事件日志 + 可选快照」恢复会话
//! - Counter/Archive/Gates/Enabled/Status/Summary/快照 视图
//!
//! 持久化（append + fsync、快照落盘、压缩）、时钟、ID 生成、通知、ACL/审计
//! 仍由调用方（`pact-engine` 或 `pact-wasm` 宿主）承担。

use pact_analysis as analysis;
use pact_core::{
    enabled_productions, rule_e, rule_p, rule_r, rule_s, termination_holds, ArchiveEntry,
    CounterState, Domain, ParticipantName, ProductionOption, RuleError, ThingRef, Token, TokenId,
    TypeName,
};
use pact_parser::{load_domain, Problem};
use pact_store_model::{apply_event, Event, ModelError, Snapshot};
use pact_views::{AnalysisReport, ArchiveView, CounterView, DomainSummary, GateClause, StatusView};
use std::collections::BTreeMap;

/// 内核错误：只覆盖纯语义/模型层失败；实例与声明注册表错误由调用方表达。
#[derive(Debug)]
pub enum KernelError {
    Parse(Problem),
    Rule(RuleError),
    Model(ModelError),
    /// 日志无 `DomainLoaded` 且无快照（历史状态不可恢复）。
    Recovery(String),
    /// 幂等键冲突：同一 `request_id` 已用于另一 token 的消费。
    IdempotencyConflict {
        request_id: String,
        existing: TokenId,
        requested: TokenId,
    },
}

impl std::fmt::Display for KernelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KernelError::Parse(e) => write!(f, "域声明无效：{e:?}"),
            KernelError::Rule(e) => write!(f, "规则拒绝：{e:?}"),
            KernelError::Model(e) => write!(f, "事件模型失败：{e}"),
            KernelError::Recovery(msg) => write!(f, "恢复失败：{msg}"),
            KernelError::IdempotencyConflict {
                request_id,
                existing,
                requested,
            } => write!(
                f,
                "幂等键 {request_id} 已用于消费 token {existing}，与本次请求的 {requested} 冲突"
            ),
        }
    }
}

impl std::error::Error for KernelError {}

impl From<Problem> for KernelError {
    fn from(e: Problem) -> Self {
        KernelError::Parse(e)
    }
}

impl From<RuleError> for KernelError {
    fn from(e: RuleError) -> Self {
        KernelError::Rule(e)
    }
}

impl From<ModelError> for KernelError {
    fn from(e: ModelError) -> Self {
        KernelError::Model(e)
    }
}

/// 实例会话：原生 `InstanceSession` 与 WASM 宿主 `InstanceSession` 的公共内核。
///
/// 字段公开以便调用方通过 `Deref` 读取/更新；不变量：
/// - `next_seq >= last_seq + 1`
/// - `dl_seq` 为本实例最近一次 `DomainLoaded` 的 seq
#[derive(Debug, Clone)]
pub struct Session {
    /// 实例创建（或最近一次重置）时的声明原文；压缩后由快照承载。
    pub declaration: String,
    pub domain: Domain,
    pub state: CounterState,
    pub last_seq: u64,
    /// 下一个可分配 seq。WASM 在 `prepare` 时预留、`abort` 时回收；原生恒为 `last_seq + 1`。
    pub next_seq: u64,
    pub dl_seq: u64,
    /// 当前 Rule-T 判定（信息性，不锁定实例）。
    pub terminated: bool,
    /// Rule-E 幂等表：request_id → (目标 token id, 首次归档结果)。
    pub consumed_requests: BTreeMap<String, (TokenId, ArchiveEntry)>,
}

/// 新建实例的结果：待持久化的 `DomainLoaded` + 初始会话。
#[derive(Debug, Clone)]
pub struct FreshSession {
    pub event: Event,
    pub session: Session,
}

/// 一次动作的纯准备结果：事件（seq 取 `next_seq`）+ 命令结果。
#[derive(Debug, Clone)]
pub struct Prepared<T> {
    pub event: Event,
    pub result: T,
}

/// `prepare_consume` 的结果：幂等命中（无事件）或新事件。
#[derive(Debug, Clone)]
pub enum ConsumePreparation {
    Replayed(ArchiveEntry),
    /// Boxed：`Prepared` 含完整 `Event`，避免枚举整体按大 variant 尺寸分配。
    Prepared(Box<Prepared<ArchiveEntry>>),
}

/// 提交事件的纯结果：Rule-T 候选 seq（尚未持久化）+ 提交后状态。
#[derive(Debug, Clone)]
pub struct CommitOutcome {
    /// 若本次提交首次满足 Rule-T，返回待追加 `Terminated` 事件的 seq。
    pub terminated_seq: Option<u64>,
    pub status: StatusView,
}

impl Session {
    /// 创建/重置实例：解析声明并生成 `DomainLoaded`（seq 由调用方给出）。
    pub fn fresh(
        name: &str,
        declaration: &str,
        ts: &str,
        seq: u64,
    ) -> Result<FreshSession, KernelError> {
        let domain = load_domain(declaration)?;
        let event = Event::DomainLoaded {
            seq,
            ts: ts.to_owned(),
            name: name.to_owned(),
            declaration: declaration.to_owned(),
        };
        let terminated = termination_holds(&domain, &CounterState::new());
        Ok(FreshSession {
            event,
            session: Session {
                declaration: declaration.to_owned(),
                domain,
                state: CounterState::new(),
                last_seq: seq,
                next_seq: seq + 1,
                dl_seq: seq,
                terminated,
                consumed_requests: BTreeMap::new(),
            },
        })
    }

    /// 从「事件日志 + 可选快照」恢复；算法与原生 `recover_instance` 一致：
    /// 日志有 `DomainLoaded` → 以它为声明与重置基线；前缀已压缩 → 以快照为锚。
    pub fn recover(
        events: &[Event],
        snapshot: Option<&Snapshot>,
        base_seq: u64,
    ) -> Result<Session, KernelError> {
        let snap_seq = snapshot.map(|s| s.seq).unwrap_or(0);
        let last_seq = events
            .last()
            .map(Event::seq)
            .unwrap_or(0)
            .max(base_seq)
            .max(snap_seq);

        let dl_idx = events
            .iter()
            .rposition(|e| matches!(e, Event::DomainLoaded { .. }));
        let (declaration, dl_seq, snapshot_is_anchor) = match dl_idx {
            Some(idx) => match &events[idx] {
                Event::DomainLoaded {
                    declaration, seq, ..
                } => (declaration.clone(), *seq, false),
                _ => unreachable!(),
            },
            None => match snapshot {
                Some(s) => (s.declaration.clone(), s.dl_seq, true),
                None => {
                    return Err(KernelError::Recovery(
                        "日志无 DomainLoaded 且无可用快照：历史状态不可恢复".to_owned(),
                    ))
                }
            },
        };
        let domain = load_domain(&declaration)?;

        let (mut state, state_base) = match (snapshot, snapshot_is_anchor) {
            (Some(s), true) => (s.state.clone(), s.seq),
            (Some(s), false) if s.seq >= dl_seq && s.seq <= last_seq => (s.state.clone(), s.seq),
            _ => (CounterState::new(), dl_seq),
        };
        for ev in events.iter().filter(|e| e.seq() > state_base) {
            apply_event(&mut state, ev)?;
        }

        // 幂等表：快照锚以快照为基，补快照之后的事件；否则从最近一次重置之后重建。
        let idem_floor = if snapshot_is_anchor {
            state_base
        } else {
            dl_seq
        };
        let mut consumed_requests: BTreeMap<String, (TokenId, ArchiveEntry)> = if snapshot_is_anchor
        {
            snapshot
                .map(|s| s.consumed_requests.clone())
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        for ev in events.iter().filter(|e| e.seq() > idem_floor) {
            if let Event::Consumed {
                token,
                by,
                ts,
                request_id: Some(rid),
                ..
            } = ev
            {
                consumed_requests.insert(
                    rid.clone(),
                    (
                        token.id.clone(),
                        ArchiveEntry {
                            token: token.clone(),
                            exhausted_by: by.clone(),
                            exhausted_at: ts.clone(),
                        },
                    ),
                );
            }
        }

        let terminated = termination_holds(&domain, &state);
        Ok(Session {
            declaration,
            domain,
            state,
            last_seq,
            next_seq: last_seq + 1,
            dl_seq,
            terminated,
            consumed_requests,
        })
    }

    /// 「实例非空」：最后一次 `DomainLoaded` 之后已有事件。
    pub fn is_occupied(&self) -> bool {
        self.last_seq > self.dl_seq
    }

    // —— 动作准备（不改变已提交状态） ——

    pub fn prepare_seed(
        &self,
        participant: &ParticipantName,
        ty: &TypeName,
        thing_ref: ThingRef,
        token_id: TokenId,
        ts: &str,
    ) -> Result<Prepared<Token>, KernelError> {
        let token = rule_s(
            &self.domain,
            participant,
            ty,
            thing_ref,
            token_id,
            ts.to_owned(),
        )?;
        Ok(Prepared {
            event: Event::Seeded {
                seq: self.next_seq,
                ts: ts.to_owned(),
                token: token.clone(),
            },
            result: token,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_produce(
        &self,
        participant: &ParticipantName,
        ty: &TypeName,
        predecessors: &[TokenId],
        thing_ref: ThingRef,
        token_id: TokenId,
        ts: &str,
    ) -> Result<Prepared<Token>, KernelError> {
        let token = rule_p(
            &self.domain,
            &self.state,
            participant,
            ty,
            predecessors,
            thing_ref,
            token_id,
            ts.to_owned(),
        )?;
        Ok(Prepared {
            event: Event::Produced {
                seq: self.next_seq,
                ts: ts.to_owned(),
                token: token.clone(),
            },
            result: token,
        })
    }

    pub fn prepare_consume(
        &self,
        participant: &ParticipantName,
        token_id: &TokenId,
        request_id: Option<&str>,
        ts: &str,
    ) -> Result<ConsumePreparation, KernelError> {
        if let Some(rid) = request_id {
            if let Some((existing, entry)) = self.consumed_requests.get(rid) {
                if existing == token_id {
                    return Ok(ConsumePreparation::Replayed(entry.clone()));
                }
                return Err(KernelError::IdempotencyConflict {
                    request_id: rid.to_owned(),
                    existing: existing.clone(),
                    requested: token_id.clone(),
                });
            }
        }
        let entry = rule_e(
            &self.domain,
            &self.state,
            participant,
            token_id,
            ts.to_owned(),
        )?;
        Ok(ConsumePreparation::Prepared(Box::new(Prepared {
            event: Event::Consumed {
                seq: self.next_seq,
                ts: ts.to_owned(),
                token: entry.token.clone(),
                by: participant.clone(),
                request_id: request_id.map(str::to_owned),
            },
            result: entry,
        })))
    }

    /// Rule-R：只读校验，无事件、无状态变更。
    pub fn read(
        &self,
        participant: &ParticipantName,
        token_id: &TokenId,
    ) -> Result<Token, KernelError> {
        Ok(rule_r(&self.domain, &self.state, participant, token_id)?)
    }

    // —— 提交与 seq 管理 ——

    /// WASM 两阶段提交：`prepare` 预留 seq（`next_seq`），`abort` 回收。
    pub fn mark_prepared(&mut self, seq: u64) {
        self.next_seq = seq + 1;
    }

    /// 放弃尚未持久化的准备事务；仅当该 seq 仍是最新预留时回收。
    pub fn abort_prepared(&mut self, seq: u64) {
        if self.next_seq == seq + 1 {
            self.next_seq = seq;
        }
    }

    /// 应用已持久化事件：更新状态、seq、幂等表，并计算 Rule-T 候选。
    ///
    /// `Terminated` 事件的实际持久化由调用方完成（原生在 `post_commit`，
    /// WASM 在宿主 `commit`）；成功追加后调用 [`Session::record_terminated`]。
    pub fn commit(&mut self, event: &Event) -> Result<CommitOutcome, KernelError> {
        apply_event(&mut self.state, event)?;
        self.last_seq = event.seq();
        self.next_seq = self.last_seq + 1;
        if let Event::Consumed {
            token,
            by,
            ts,
            request_id: Some(rid),
            ..
        } = event
        {
            self.consumed_requests.insert(
                rid.clone(),
                (
                    token.id.clone(),
                    ArchiveEntry {
                        token: token.clone(),
                        exhausted_by: by.clone(),
                        exhausted_at: ts.clone(),
                    },
                ),
            );
        }

        let now_terminated = termination_holds(&self.domain, &self.state);
        let terminated_seq = if now_terminated && !self.terminated {
            Some(self.next_seq)
        } else {
            None
        };
        self.terminated = now_terminated;
        Ok(CommitOutcome {
            terminated_seq,
            status: self.status(),
        })
    }

    /// 记录已成功持久化的 `Terminated` 事件（无状态效果，仅推进 seq）。
    pub fn record_terminated(&mut self, seq: u64) {
        self.last_seq = seq;
        self.next_seq = seq + 1;
    }

    // —— 视图 ——

    pub fn counter(&self, filter: Option<&TypeName>) -> CounterView {
        let tokens: Vec<Token> = self
            .state
            .counter
            .values()
            .filter(|t| filter.is_none_or(|f| t.ty == *f))
            .cloned()
            .collect();
        CounterView {
            size: tokens.len(),
            tokens,
        }
    }

    pub fn archive(&self, filter: Option<&TypeName>) -> ArchiveView {
        let entries: Vec<ArchiveEntry> = self
            .state
            .archive
            .iter()
            .filter(|e| filter.is_none_or(|f| e.token.ty == *f))
            .cloned()
            .collect();
        ArchiveView {
            size: entries.len(),
            entries,
        }
    }

    pub fn gates(&self, participant: &ParticipantName) -> Vec<GateClause> {
        self.domain
            .gates
            .iter()
            .filter(|((p, _, _), _)| p == participant)
            .map(|((_, action, ty), cond)| GateClause {
                action: *action,
                ty: ty.clone(),
                condition: cond.clone(),
            })
            .collect()
    }

    pub fn enabled(&self, participant: &ParticipantName) -> Vec<ProductionOption> {
        enabled_productions(&self.domain, &self.state, participant)
    }

    pub fn status(&self) -> StatusView {
        StatusView {
            counter_size: self.state.counter.len(),
            archive_size: self.state.archive.len(),
            terminated: self.terminated,
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot::new(
            self.last_seq,
            self.dl_seq,
            self.declaration.clone(),
            self.state.clone(),
            self.consumed_requests.clone(),
        )
    }

    pub fn summary(&self, name: &str, instance: &str) -> DomainSummary {
        DomainSummary {
            name: name.to_owned(),
            instance: instance.to_owned(),
            types: self.domain.types.len(),
            gates: self.domain.gates.len(),
            seed_authority: self.domain.seed_authority.len(),
            last_seq: self.last_seq,
        }
    }
}

/// 六项静态分析聚合（论文 Appendix A.1–A.6）；A.5 置于首位。
pub fn analysis_report(domain: &Domain) -> AnalysisReport {
    AnalysisReport {
        precondition_wf: analysis::precondition_wf(domain),
        reachability: analysis::reachability(domain),
        gate_coverage: analysis::gate_coverage(domain),
        consumption_reachability: analysis::consumption_reachability(domain),
        structural_deadlock: analysis::structural_deadlock(domain),
        stranding_risk: analysis::stranding_risk(domain),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DECL: &str = r#"
types { requirement; design }
preconditions { requirement → design }
gates {
    architect: reads requirement, consumes requirement, produces design;
    user: consumes design
}
seed_authority { external: seeds requirement }
"#;

    fn key() -> (&'static str, &'static str) {
        ("pipeline", "default")
    }

    fn names() -> (ParticipantName, TypeName, TypeName) {
        (
            ParticipantName("external".into()),
            TypeName("requirement".into()),
            TypeName("design".into()),
        )
    }

    #[test]
    fn fresh_allocates_contiguous_seq() {
        let (name, instance) = key();
        let fresh = Session::fresh(name, DECL, "T0", 1).unwrap();
        assert_eq!(fresh.event.seq(), 1);
        assert_eq!(fresh.session.next_seq, 2);
        assert_eq!(fresh.session.summary(name, instance).last_seq, 1);
        assert!(!fresh.session.is_occupied());
    }

    #[test]
    fn commit_applies_state_and_reports_termination_once() {
        let (name, _) = key();
        let mut s = Session::fresh(name, DECL, "T0", 1).unwrap().session;
        let (external, requirement, _) = names();
        let seed = s
            .prepare_seed(
                &external,
                &requirement,
                ThingRef::none(),
                TokenId("seq-0".into()),
                "T1",
            )
            .unwrap();
        s.mark_prepared(seed.event.seq());
        let out = s.commit(&seed.event).unwrap();
        assert_eq!(out.terminated_seq, None, "有可生产项时不应终止");
        assert_eq!(s.last_seq, 2);

        let entry = s
            .prepare_consume(
                &ParticipantName("architect".into()),
                &TokenId("seq-0".into()),
                Some("rid-1"),
                "T2",
            )
            .unwrap();
        let ConsumePreparation::Prepared(entry) = entry else {
            panic!("首次消费不应幂等命中");
        };
        let out = s.commit(&entry.event).unwrap();
        assert_eq!(out.terminated_seq, Some(4));
        s.record_terminated(4);
        assert!(s.status().terminated);
        assert_eq!(s.last_seq, 4);

        // 幂等命中：同键同 token 返回首次归档，不产生事件
        let replay = s
            .prepare_consume(
                &ParticipantName("architect".into()),
                &TokenId("seq-0".into()),
                Some("rid-1"),
                "T3",
            )
            .unwrap();
        let ConsumePreparation::Replayed(replayed) = replay else {
            panic!("重复 request_id 应命中幂等表");
        };
        assert_eq!(replayed, entry.result);

        // 同键异 token → IdempotencyConflict
        let err = s
            .prepare_consume(
                &ParticipantName("architect".into()),
                &TokenId("seq-9".into()),
                Some("rid-1"),
                "T4",
            )
            .unwrap_err();
        assert!(matches!(err, KernelError::IdempotencyConflict { .. }));
    }

    #[test]
    fn recovery_from_snapshot_matches_replay() {
        let (name, _) = key();
        let mut s = Session::fresh(name, DECL, "T0", 1).unwrap().session;
        let (external, requirement, design) = names();
        let seed = s
            .prepare_seed(
                &external,
                &requirement,
                ThingRef::none(),
                TokenId("seq-0".into()),
                "T1",
            )
            .unwrap();
        s.commit(&seed.event).unwrap();
        let prod = s
            .prepare_produce(
                &ParticipantName("architect".into()),
                &design,
                &[TokenId("seq-0".into())],
                ThingRef::none(),
                TokenId("seq-1".into()),
                "T2",
            )
            .unwrap();
        s.commit(&prod.event).unwrap();
        let snap = s.snapshot();

        // 快照锚 + 空日志（模拟 keep_last=0 压缩）
        let recovered = Session::recover(&[], Some(&snap), snap.seq).unwrap();
        assert_eq!(recovered.status(), s.status());
        assert_eq!(recovered.counter(None), s.counter(None));
        assert_eq!(recovered.declaration, s.declaration);
    }
}
