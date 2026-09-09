//! PACT 域静态分析（论文 Appendix A，A.1–A.6）。
//!
//! 全部为纯函数：输入 `&Domain`，输出结构化报告（非仅布尔），供引擎层 /
//! 工具链透传为线上 JSON。分析不执行系统（论文 Chapter 5：performed on the
//! static domain declaration, without executing the system）。
//!
//! | 分析 | 问题 | 复杂度（论文 Appendix D） |
//! |------|------|------|
//! | A.1 可达性 | 类型 T 能否从种子权威类型沿前置图到达 | O(\|Types\| + \|Pre\|) |
//! | A.2 生产覆盖 | 每个非源类型是否至少有一个 producer | O(\|Types\| + \|Pre\| + \|GateEntries\|) |
//! | A.3 消费可达性 | 每个类型（含源类型）是否至少有一个 consumer | O(\|GateEntries\|) |
//! | A.4 结构死锁 | 前置图是否含无种子入口的强连通分量 | O(\|Types\| + \|Pre\|) |
//! | A.5 良构性 | §3.1 六项约束是否满足；Seed/Produce 重叠警告 | O(\|Types\| + \|Pre\| + \|GateEntries\|) |
//! | A.6 搁浅风险 | via 见证能否先于依赖动作被消费 | O(\|GateConditions\| × \|Types\|) 无环；有环保守 |
//!
//! A.5 是其余分析的前提（论文 Appendix D）：违规会破坏 A.1–A.4、A.6 的假设，
//! 故聚合报告把它放在首位，须先修正违规再解读其余结果。

use pact_core::{
    is_source_type, validate_domain, Action, Domain, DomainError, GateExpr, ParticipantName,
    TypeName,
};
use petgraph::algo::{kosaraju_scc, toposort};
use petgraph::graphmap::DiGraphMap;
use petgraph::Direction;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

// ============ A.1 可达性 ============

/// A.1 报告：种子权威类型、从种子可达的类型集合、不可达类型清单。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReachabilityReport {
    /// { T' : ∃ p, (p, T') ∈ SeedAuthority }
    pub seed_types: BTreeSet<TypeName>,
    /// 从 seed_types 沿前置图（前驱 → 后继方向）可达的全部类型（含 seed_types 自身）。
    pub reachable: BTreeSet<TypeName>,
    /// 声明但不可达的类型（按字典序）。
    pub unreachable: Vec<TypeName>,
}

/// A.1 可达性（论文 Appendix A.1）：从种子权威类型出发，沿前置图
/// （X ∈ lhs(R) → rhs(R) 方向，即"前驱就位即可生产后继"）做 BFS 闭包。
pub fn reachability(domain: &Domain) -> ReachabilityReport {
    let g = precondition_graph(domain);
    let seed_types: BTreeSet<TypeName> = domain
        .seed_authority
        .iter()
        .map(|(_, t)| t.clone())
        .filter(|t| domain.types.contains(t))
        .collect();

    let mut reachable: BTreeSet<TypeName> = BTreeSet::new();
    let mut worklist: Vec<&TypeName> = Vec::new();
    for t in &seed_types {
        if reachable.insert(t.clone()) {
            worklist.push(t);
        }
    }
    while let Some(x) = worklist.pop() {
        for y in g.neighbors_directed(x, Direction::Outgoing) {
            if reachable.insert(y.clone()) {
                worklist.push(y);
            }
        }
    }

    let unreachable: Vec<TypeName> = domain
        .types
        .iter()
        .filter(|t| !reachable.contains(*t))
        .cloned()
        .collect();
    ReachabilityReport {
        seed_types,
        reachable,
        unreachable,
    }
}

/// A.1 单目标查询：类型 T 是否从任一种子权威类型可达。
pub fn is_reachable(domain: &Domain, target: &TypeName) -> bool {
    reachability(domain).reachable.contains(target)
}

// ============ A.2 生产 gate 覆盖 ============

/// A.2 Gate Coverage（论文 Appendix A.2）：无任何 producer 的**非源类型**清单。
/// 源类型排除（经 SeedAuthority 进入，不经 Rule-P，论文 rev1 §A.2 Note）。
pub fn gate_coverage(domain: &Domain) -> Vec<TypeName> {
    let produced: BTreeSet<&TypeName> = domain
        .gates
        .keys()
        .filter(|(_, a, _)| *a == Action::Produce)
        .map(|(_, _, t)| t)
        .collect();
    domain
        .types
        .iter()
        .filter(|t| !is_source_type(domain, t) && !produced.contains(*t))
        .cloned()
        .collect()
}

// ============ A.3 消费可达性 ============

/// A.3 Consumption Reachability（论文 Appendix A.3）：无任何 consumer 的类型清单
/// （含源类型）。未消费 token 是潜在技术债（论文 Part II §1.6）。
pub fn consumption_reachability(domain: &Domain) -> Vec<TypeName> {
    let consumed: BTreeSet<&TypeName> = domain
        .gates
        .keys()
        .filter(|(_, a, _)| *a == Action::Consume)
        .map(|(_, _, t)| t)
        .collect();
    domain
        .types
        .iter()
        .filter(|t| !consumed.contains(*t))
        .cloned()
        .collect()
}

// ============ A.4 结构死锁 ============

/// A.4 报告：一个无种子入口且不可外部馈入的强连通分量。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadlockComponent {
    /// SCC 内类型（字典序）。
    pub types: Vec<TypeName>,
}

/// A.4 结构死锁（论文 Appendix A.4）：前置图 Tarjan/Kosaraju SCC，
/// 对每个非平凡 SCC 检查：
/// 1. SCC 内有类型出现在 SeedAuthority → 可种子化，跳过；
/// 2. 有外部入边（X ∉ C → T ∈ C）且 X 从种子可达 → 可外部馈入，跳过；
///
/// 否则报告死锁分量。
pub fn structural_deadlock(domain: &Domain) -> Vec<DeadlockComponent> {
    let g = precondition_graph(domain);
    let reach = reachability(domain).reachable;

    let mut out = Vec::new();
    for scc in kosaraju_scc(&g) {
        // 单点无自环 → 平凡分量，跳过
        if scc.len() == 1 {
            let t = scc[0];
            let self_loop = domain
                .preconditions
                .get(t)
                .is_some_and(|preds| preds.contains(t));
            if !self_loop {
                continue;
            }
        }
        let members: BTreeSet<&TypeName> = scc.iter().copied().collect();
        // 种子入口
        let seedable = members
            .iter()
            .any(|t| domain.seed_authority.iter().any(|(_, st)| st == *t));
        if seedable {
            continue;
        }
        // 外部馈入：∃ 直接入边 X → T（X ∉ C，T ∈ C，X 种子可达）
        let feedable = members.iter().any(|t| {
            g.neighbors_directed(*t, Direction::Incoming)
                .any(|x| !members.contains(&x) && reach.contains(x))
        });
        if feedable {
            continue;
        }
        out.push(DeadlockComponent {
            types: members.iter().map(|t| (*t).clone()).collect(),
        });
    }
    out.sort_by(|a, b| a.types.cmp(&b.types));
    out
}

// ============ A.5 前置图与源生产良构性 ============

/// A.5 咨询性警告（论文 §1.4.3 Advisory）：非违规，不阻止加载。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WfWarning {
    /// SEED_PRODUCE_OVERLAP：同一参与者对同一**非源**类型既持 SeedAuthority
    /// 又持 produces gate。规范上安全（seed 实例可作前驱被消费，produce 路径
    /// 产出新实例），但通常是建模混淆的信号，应复核意图。
    SeedProduceOverlap {
        participant: ParticipantName,
        ty: TypeName,
    },
}

impl std::fmt::Display for WfWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WfWarning::SeedProduceOverlap { participant, ty } => {
                write!(f, "SEED_PRODUCE_OVERLAP: {participant} 对非源类型 {ty} 同时持有 seed 权威与 produces gate")
            }
        }
    }
}

/// A.5 报告（论文 Appendix A.5 的三元输出 Violations / Warnings / WF(D)）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreconditionWfReport {
    /// WF(D)：无违规即良构。
    pub well_formed: bool,
    /// 良构违规。复用 core::DomainError 单一分类（与 `:load` 的错误同源）。
    /// 其中 DuplicatePrecondRule / DuplicatePrecondLhs 在 `&Domain` 输入下由
    /// BTreeMap 键唯一与 BTreeSet 去重结构性排除，其强制点在 build_domain。
    pub violations: Vec<DomainError>,
    pub warnings: Vec<WfWarning>,
}

/// A.5 Precondition & Source-Production Well-Formedness（论文 Appendix A.5）：
/// 校验 §3.1 六项良构约束，并给出 Seed/Produce 重叠警告。
/// 约束检查委托 core::validate_domain（单一判定源，避免逻辑重复）；
/// 本分析是其余分析的前提（Appendix D：A.5 应先于 A.1–A.4、A.6 运行，
/// 违规会破坏它们的假设）。
pub fn precondition_wf(domain: &Domain) -> PreconditionWfReport {
    let violations = validate_domain(domain).err().unwrap_or_default();

    let produced: BTreeSet<&TypeName> = domain
        .gates
        .keys()
        .filter(|(_, a, _)| *a == Action::Produce)
        .map(|(_, _, t)| t)
        .collect();
    let warnings = domain
        .seed_authority
        .iter()
        .filter(|(_, t)| !is_source_type(domain, t) && produced.contains(&t))
        .map(|(p, t)| WfWarning::SeedProduceOverlap {
            participant: p.clone(),
            ty: t.clone(),
        })
        .collect();

    PreconditionWfReport {
        well_formed: violations.is_empty(),
        violations,
        warnings,
    }
}

// ============ A.6 搁浅风险 ============

/// A.6 报告：一条搁浅风险（论文四元组 (T_b, T_a, p, q) + 拓扑标记）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrandingRisk {
    /// 持有 `via T_a` 条件子句的类型（被守护类型）。
    pub ty_b: TypeName,
    /// 见证类型（T_a 是 T_b 的直接前驱）。
    pub ty_a: TypeName,
    /// 持有 via 条件子句的参与者。
    pub via_holder: ParticipantName,
    /// T_a 的消费者。
    pub consumer: ParticipantName,
    /// true = 循环拓扑下的保守报告（POTENTIAL_STRANDING）；
    /// false = 无环拓扑的确定性报告（STRANDING_RISK）。
    pub potential: bool,
}

/// A.6 搁浅风险（论文 Appendix A.6）：
/// 对每个 read/consume 子句中的 `via T_a` 条件（含复合表达式内的嵌套 Via 原子）：
/// 1. T_a 必须是 T_b 的**直接**前驱（间接祖先的 via 永不可满足，无风险）；
/// 2. T_a 存在消费者（consumes T_a gate）；
///
/// 则对每个消费者 q 报告 (T_b, T_a, p, q)。GateExpr 无法表达时间顺序
/// （§1.7.3 局限），风险结构性存在；无环拓扑精确判定，有环保守判定。
pub fn stranding_risk(domain: &Domain) -> Vec<StrandingRisk> {
    let g = precondition_graph(domain);
    let cyclic = toposort(&g, None).is_err();

    let mut seen: BTreeSet<(TypeName, TypeName, ParticipantName, ParticipantName)> =
        BTreeSet::new();
    for ((p, action, ty_b), cond) in &domain.gates {
        // produces 子句无条件（§3.1）；仅 read/consume 携带条件
        if !matches!(action, Action::Read | Action::Consume) {
            continue;
        }
        let Some(expr) = cond else { continue };
        let mut vias = BTreeSet::new();
        via_atoms(expr, &mut vias);
        if vias.is_empty() {
            continue;
        }
        let direct_preds = domain.preconditions.get(ty_b);
        for ty_a in vias {
            // T_a 须为 T_b 的直接前驱（论文 Critical semantic note）
            if !direct_preds.is_some_and(|ps| ps.contains(&ty_a)) {
                continue;
            }
            // T_a 的消费者集合
            for (q, a, t) in domain.gates.keys() {
                if *a == Action::Consume && *t == ty_a {
                    seen.insert((ty_b.clone(), ty_a.clone(), p.clone(), q.clone()));
                }
            }
        }
    }

    seen.into_iter()
        .map(|(ty_b, ty_a, via_holder, consumer)| StrandingRisk {
            ty_b,
            ty_a,
            via_holder,
            consumer,
            potential: cyclic,
        })
        .collect()
}

// ============ 内部辅助 ============

/// 前置图 `DiGraphMap<&TypeName, ()>`：边 X → T 当且仅当 X ∈ Pre(T)
/// （X 是 T 的直接前驱；"前驱就位即可生产后继"方向）。
/// 节点为全部声明类型（良构域引用封闭；未声明引用的边被跳过）。
fn precondition_graph(domain: &Domain) -> DiGraphMap<&TypeName, ()> {
    let mut g = DiGraphMap::new();
    for t in &domain.types {
        g.add_node(t);
    }
    for (rhs, preds) in &domain.preconditions {
        for p in preds {
            if domain.types.contains(p) && domain.types.contains(rhs) {
                g.add_edge(p, rhs, ());
            }
        }
    }
    g
}

/// 递归提取 GateExpr 中全部 Via 原子的类型名（A.6 遍历用）。
fn via_atoms(expr: &GateExpr, out: &mut BTreeSet<TypeName>) {
    match expr {
        GateExpr::Via(t) => {
            out.insert(t.clone());
        }
        GateExpr::Exists(_) | GateExpr::Count { .. } => {}
        GateExpr::And(a, b) | GateExpr::Or(a, b) => {
            via_atoms(a, out);
            via_atoms(b, out);
        }
        GateExpr::Not(e) => via_atoms(e, out),
    }
}
