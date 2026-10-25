//! 引擎查询视图（线上格式设计 §6.5 / §9；三接口共享）。
//!
//! 纯 serde 类型，无 IO、无 tokio 依赖：原生 `pact-engine` 与 `pact-wasm`
//! 共用同一份视图定义，保证三接口与 WASM 门面输出同构。

use pact_analysis::{DeadlockComponent, PreconditionWfReport, ReachabilityReport, StrandingRisk};
use pact_core::{Action, ArchiveEntry, GateExpr, Token, TypeName};
use serde::{Deserialize, Serialize};

/// Counter 查询（§6.5 `GET /domains/{d}/counter?type=`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterView {
    pub tokens: Vec<Token>,
    pub size: usize,
}

/// Archive 查询。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchiveView {
    pub entries: Vec<ArchiveEntry>,
    pub size: usize,
}

/// 状态概要（§6.5 `GET /domains/{d}/status`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusView {
    pub counter_size: usize,
    pub archive_size: usize,
    pub terminated: bool,
}

/// 参与者的一个 gate 子句（`pact_my_gates`，agent 自省）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateClause {
    pub action: Action,
    #[serde(rename = "type")]
    pub ty: TypeName,
    /// via/when 条件（None = 无条件子句）。
    pub condition: Option<GateExpr>,
}

/// 域加载结果概要（定义键 + 实例键）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainSummary {
    pub name: String,
    /// 实例键（默认 `default`）。
    pub instance: String,
    pub types: usize,
    pub gates: usize,
    pub seed_authority: usize,
    pub last_seq: u64,
}

/// 定义键及其实例清单（`GET /domains` 线上格式，D27）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainListing {
    pub name: String,
    /// 该声明下已存在的实例名（排序去重）。
    pub instances: Vec<String>,
    /// 短路由当前解析到的实例（活动实例别名）；`None` = 回退 `default`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
}

/// 状态交接结果（`POST /domains/{d}/handoff`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffOutcome {
    pub domain: String,
    pub source_instance: String,
    pub target_instance: String,
    /// 导出时源实例的最后事件序号。
    pub source_seq: u64,
    /// 目标实例交接基线事件的 seq。
    pub target_seq: u64,
    pub counter: usize,
    pub archive: usize,
    pub consumed_requests: usize,
    /// 交接后短路由实际解析到的实例（激活失败时可能仍为源实例或旧指针）。
    pub active_instance: String,
    /// 是否已成功切换活动实例指针。
    pub activated: bool,
}

/// 日志前缀压缩结果（D25 子项 1；`POST …/compact` 响应）。
///
/// 语义：以快照 `snapshot_seq` 为提交点，日志中 `seq <= base_seq` 的行被丢弃；
/// 保留 `retained` 行、丢弃 `discarded` 行。压缩后实例状态由快照承载。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionOutcome {
    pub domain: String,
    pub instance: String,
    /// 快照已覆盖到的最后事件 seq（压缩前日志末尾 seq）。
    pub snapshot_seq: u64,
    /// 压缩后日志保留水位（可从日志丢弃的事件上界）。
    pub base_seq: u64,
    /// 日志中保留的事件行数。
    pub retained_events: u64,
    /// 本次丢弃的事件行数。
    pub discarded_events: u64,
}

/// 六项静态分析聚合报告（论文 Appendix A.1–A.6）。
/// A.5 置于首位：它是其余分析的前提（Appendix D），须先修正违规再解读其余结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisReport {
    /// A.5：§3.1 良构约束违规 + Seed/Produce 重叠警告。
    pub precondition_wf: PreconditionWfReport,
    pub reachability: ReachabilityReport,
    /// A.2：无任何 producer 的非源类型。
    pub gate_coverage: Vec<TypeName>,
    /// A.3：无任何 consumer 的类型。
    pub consumption_reachability: Vec<TypeName>,
    pub structural_deadlock: Vec<DeadlockComponent>,
    pub stranding_risk: Vec<StrandingRisk>,
}
