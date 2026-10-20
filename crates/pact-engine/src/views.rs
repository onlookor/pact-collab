//! 引擎查询视图（线上格式设计 §6.5 / §9；三接口共享）。

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
