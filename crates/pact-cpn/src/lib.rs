//! PACT 域声明 → CPN Tools 彩色 Petri 网编码（论文 §4.5 / 附录 B）。
//!
//! 编码只针对**单个 Counter 实例**（论文 §4.5 Scope、§B.2 Scope）：本 crate
//! 不读取运行期状态，也不跨实例共享库所/迁移；`instance` 仅用于给生成模型的
//! 库所、迁移与页面名加命名空间前缀，确保不同实例的编码互不冲突。
//!
//! 编码要点：
//! - 每个类型一个库所、一个 Archive 库所；`TokenId` 作为颜色分量承载溯源；
//! - 生产迁移对前驱使用 test arc（输入绑定 + 同变量输出），不消费前驱；
//! - 读迁移自环；消费迁移消耗目标并输出到 Archive；
//! - 种子迁移无输入弧；
//! - `via T` 用见证库所 test arc + `List.exists` guard；
//! - `and`/`or`/`not` 先化到 DNF：`or` 展开为多条迁移，`not` 下推为
//!   `count T <= k`，再用互补库所编码（需调用方提供类型上界）；
//! - `not via` 无法用互补库所表达（见证是实例级关系而非计数），显式报错。
//!
//! 输出为 CPN Tools 4.x XML（`format='6'`），可直接由 CPN Tools 打开。

mod formula;
mod xml;

use formula::Term;
use pact_core::{is_source_type, Action, Domain, ParticipantName, TypeName};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// 生成选项。
#[derive(Debug, Clone)]
pub struct CpnOptions {
    /// 实例键（论文 `Counter instance`，见 D27）；仅用于命名空间前缀。
    pub instance: String,
    /// 互补库所编码所需的各类型同时存在 token 数的已知上界。
    pub bounds: BTreeMap<TypeName, u64>,
    /// CPN 页面名（缺省 `PACT_<instance>`）。
    pub page_name: Option<String>,
}

impl Default for CpnOptions {
    fn default() -> Self {
        Self {
            instance: "default".into(),
            bounds: BTreeMap::new(),
            page_name: None,
        }
    }
}

/// 生成统计（库所/迁移/弧计数，便于核对论文 §B.6 的线性规模）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpnStats {
    pub instance: String,
    pub places: usize,
    pub transitions: usize,
    pub arcs: usize,
    pub production_transitions: usize,
    pub read_transitions: usize,
    pub consume_transitions: usize,
    pub seed_transitions: usize,
    pub complementary_places: usize,
    /// 条件恒假的迁移（保留迁移、guard=false；不计入上述动作计数）。
    pub disabled_transitions: usize,
}

/// 生成结果。
#[derive(Debug, Clone)]
pub struct CpnModel {
    pub xml: String,
    pub stats: CpnStats,
}

/// 生成错误。
/// 其中 `MissingBound` / `NegatedVia` 是**工具层**表达边界，不是范式边界；
/// 处置与建模损失见 `docs/expressiveness-boundaries.md` B23/B24。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CpnError {
    /// 出现 `count T <= k` 但未提供 T 的上界。
    MissingBound { ty: TypeName },
    /// `not via T` 无法用互补库所编码。
    NegatedVia { ty: TypeName },
    /// 条件 DNF 展开过大或计数溢出。
    ConditionTooComplex { reason: String },
    /// 输入域不满足生成前提（源类型生产、生产携带条件等）。
    NotWellFormed { reason: String },
}

impl fmt::Display for CpnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CpnError::MissingBound { ty } => write!(
                f,
                "条件包含 `not`/`count =` 对类型 {} 的缺席或上界判断，须提供 --bound {}=N（N 为该类型同时存在的 token 数上界）",
                ty.0, ty.0
            ),
            CpnError::NegatedVia { ty } => write!(
                f,
                "`not via {}` 无法用标准 CPN 的互补库所编码（见证是实例级依赖关系，不是计数）",
                ty.0
            ),
            CpnError::ConditionTooComplex { reason } => write!(f, "条件编码失败：{reason}"),
            CpnError::NotWellFormed { reason } => write!(f, "域不满足 CPN 生成前提：{reason}"),
        }
    }
}

impl std::error::Error for CpnError {}

/// 生成单实例的 CPN 编码。
pub fn generate(domain: &Domain, options: &CpnOptions) -> Result<CpnModel, CpnError> {
    // 第一遍：条件 → DNF，并收集需要互补库所的类型。
    let mut terms_by_gate: BTreeMap<(ParticipantName, Action, TypeName), Vec<Term>> =
        BTreeMap::new();
    let mut complements: BTreeSet<TypeName> = BTreeSet::new();
    for ((participant, action, ty), cond) in &domain.gates {
        if let Some(expr) = cond {
            if *action == Action::Produce {
                return Err(CpnError::NotWellFormed {
                    reason: format!(
                        "参与者 {} 的 produces {} 携带条件（违反 §3.1）",
                        participant.0, ty.0
                    ),
                });
            }
            let terms = formula::terms(expr, &options.bounds)?;
            for term in &terms {
                complements.extend(term.at_most.keys().cloned());
            }
            terms_by_gate.insert((participant.clone(), *action, ty.clone()), terms);
        }
    }

    // 第二遍：构造网结构。
    let mut b = Builder::new(
        options.instance.clone(),
        options.bounds.clone(),
        options.page_name.clone(),
    );

    for ty in &domain.types {
        b.add_place(
            PlaceKey::Type(ty.clone()),
            format!("p_{}", ident(&ty.0)),
            "PACT_TOKEN",
            "empty".into(),
        );
    }
    b.add_place(
        PlaceKey::Archive,
        "p_archive".into(),
        "PACT_ARCHIVE",
        "empty".into(),
    );
    for ty in &complements {
        let bound = options
            .bounds
            .get(ty)
            .copied()
            .ok_or_else(|| CpnError::MissingBound { ty: ty.clone() })?;
        b.add_place(
            PlaceKey::Complement(ty.clone()),
            format!("p_{}_not", ident(&ty.0)),
            "PACT_COMPL",
            format!("{bound}`0"),
        );
        b.stats.complementary_places += 1;
    }

    for (participant, action, ty) in domain.gates.keys() {
        match action {
            Action::Produce => {
                if is_source_type(domain, ty) {
                    return Err(CpnError::NotWellFormed {
                        reason: format!(
                            "参与者 {} 的 produces {} 指向源类型（违反 §3.1 源生产约束）",
                            participant.0, ty.0
                        ),
                    });
                }
                b.add_production(domain, participant, ty, &complements);
            }
            Action::Read | Action::Consume => {
                let terms = terms_by_gate
                    .get(&(participant.clone(), *action, ty.clone()))
                    .cloned();
                b.add_guarded(participant, ty, *action, terms.as_deref(), &complements);
            }
        }
    }
    for (participant, ty) in &domain.seed_authority {
        b.add_seed(participant, ty, &complements);
    }

    Ok(b.finish())
}

// ============ 标识符 ============

/// CPN ML 标识符安全化：保留字母数字与下划线，其余替换为下划线。
fn ident(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        out.push('_');
    }
    out
}

// ============ 中间结构 ============

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum PlaceKey {
    Type(TypeName),
    Archive,
    Complement(TypeName),
}

struct PlaceDef {
    id: String,
    name: String,
    color: String,
    init: String,
    x: f64,
    y: f64,
}

struct TransDef {
    id: String,
    name: String,
    guard: String,
    x: f64,
    y: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    PtoT,
    TtoP,
}

struct ArcDef {
    id: String,
    from: String,
    to: String,
    expr: String,
    dir: Direction,
}

struct Builder {
    instance: String,
    bounds: BTreeMap<TypeName, u64>,
    page_name: String,
    next_id: u64,
    places: Vec<PlaceDef>,
    place_ids: BTreeMap<PlaceKey, String>,
    trans: Vec<TransDef>,
    arcs: Vec<ArcDef>,
    stats: CpnStats,
    max_predecessors: usize,
    max_witnesses: usize,
}

impl Builder {
    fn new(instance: String, bounds: BTreeMap<TypeName, u64>, page_name: Option<String>) -> Self {
        let page_name = page_name.unwrap_or_else(|| format!("PACT_{}", ident(&instance)));
        Self {
            instance,
            bounds,
            page_name,
            next_id: 0,
            places: Vec::new(),
            place_ids: BTreeMap::new(),
            trans: Vec::new(),
            arcs: Vec::new(),
            stats: CpnStats::default(),
            max_predecessors: 0,
            max_witnesses: 0,
        }
    }

    fn id(&mut self) -> String {
        self.next_id += 1;
        format!("ID{}", self.next_id)
    }

    fn scope(&self, raw: &str) -> String {
        format!("i_{}__{}", ident(&self.instance), raw)
    }

    fn add_place(&mut self, key: PlaceKey, name: String, color: &str, init: String) {
        let id = self.id();
        let idx = self.places.len();
        let name = self.scope(&name);
        self.places.push(PlaceDef {
            id: id.clone(),
            name,
            color: color.into(),
            init,
            x: 90.0 + (idx % 4) as f64 * 180.0,
            y: 80.0 + (idx / 4) as f64 * 130.0,
        });
        self.place_ids.insert(key, id);
        self.stats.places += 1;
    }

    fn place_id(&self, key: &PlaceKey) -> String {
        self.place_ids
            .get(key)
            .cloned()
            .expect("库所应在构造迁移前创建")
    }

    fn add_transition(&mut self, name: String, guard: String) -> String {
        let id = self.id();
        let idx = self.trans.len();
        self.trans.push(TransDef {
            id: id.clone(),
            name: self.scope(&name),
            guard,
            x: 700.0 + (idx % 4) as f64 * 180.0,
            y: 80.0 + (idx / 4) as f64 * 130.0,
        });
        self.stats.transitions += 1;
        id
    }

    fn arc(&mut self, key: &PlaceKey, trans_id: &str, expr: String, dir: Direction) {
        let id = self.id();
        let place_id = self.place_id(key);
        let (from, to) = match dir {
            Direction::PtoT => (place_id, trans_id.to_string()),
            Direction::TtoP => (trans_id.to_string(), place_id),
        };
        self.arcs.push(ArcDef {
            id,
            from,
            to,
            expr,
            dir,
        });
        self.stats.arcs += 1;
    }

    /// 迁移变量绑定：x 为目标 token；x1..xK 为生产前驱；y1..yM 为条件见证。
    fn add_production(
        &mut self,
        domain: &Domain,
        participant: &ParticipantName,
        ty: &TypeName,
        complements: &BTreeSet<TypeName>,
    ) {
        let preds: Vec<TypeName> = domain
            .preconditions
            .get(ty)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();
        if preds.is_empty() {
            // 源类型已在入口拦截；此分支防御程序化构造的非良构域。
            return;
        }
        let name = format!("t_{}_{}_prod", ident(&participant.0), ident(&ty.0));
        let trans_id = self.add_transition(name, String::new());

        let mut dep_ids = Vec::with_capacity(preds.len());
        for (i, pred) in preds.iter().enumerate() {
            let var = format!("x{}", i + 1);
            self.arc(
                &PlaceKey::Type(pred.clone()),
                &trans_id,
                var.clone(),
                Direction::PtoT,
            );
            self.arc(
                &PlaceKey::Type(pred.clone()),
                &trans_id,
                var.clone(),
                Direction::TtoP,
            );
            dep_ids.push(format!("#1 {var}"));
        }
        self.max_predecessors = self.max_predecessors.max(preds.len());

        let deps = if dep_ids.is_empty() {
            "[]".to_string()
        } else {
            format!("[{}]", dep_ids.join(", "))
        };
        let expr = format!(
            "(pact_next_id(), \"{}\", \"\", {deps}, pact_now())",
            escape(&participant.0)
        );
        self.arc(
            &PlaceKey::Type(ty.clone()),
            &trans_id,
            expr,
            Direction::TtoP,
        );
        self.maintain_complement_on_output(ty, &trans_id, complements);
        self.stats.production_transitions += 1;
    }

    fn add_seed(
        &mut self,
        participant: &ParticipantName,
        ty: &TypeName,
        complements: &BTreeSet<TypeName>,
    ) {
        let name = format!("t_{}_{}_seed", ident(&participant.0), ident(&ty.0));
        let trans_id = self.add_transition(name, String::new());
        let expr = format!(
            "(pact_next_id(), \"{}\", \"\", [], pact_now())",
            escape(&participant.0)
        );
        self.arc(
            &PlaceKey::Type(ty.clone()),
            &trans_id,
            expr,
            Direction::TtoP,
        );
        self.maintain_complement_on_output(ty, &trans_id, complements);
        self.stats.seed_transitions += 1;
    }

    /// 读/消费迁移；`terms = None` 表示无条件，`Some([])` 表示条件恒假。
    fn add_guarded(
        &mut self,
        participant: &ParticipantName,
        ty: &TypeName,
        action: Action,
        terms: Option<&[Term]>,
        complements: &BTreeSet<TypeName>,
    ) {
        let action_name = action.as_str();
        let base = format!(
            "t_{}_{}_{}",
            ident(&participant.0),
            ident(&ty.0),
            action_name
        );
        let list: Vec<Option<Term>> = match terms {
            Some([]) => vec![None],
            Some(ts) => ts.iter().cloned().map(Some).collect(),
            None => vec![None],
        };
        let disabled = matches!(terms, Some([]));
        let multi = list.len() > 1;

        for (idx, term) in list.iter().enumerate() {
            let name = if multi {
                format!("{base}_c{}", idx + 1)
            } else {
                base.clone()
            };
            // 目标 token 自身计入目标类型的 count，故 `count T <= 0`（T = 目标）
            // 在本迁移上恒假。
            let term_disabled =
                disabled || term.as_ref().is_some_and(|t| t.at_most.get(ty) == Some(&0));
            let mut guard_parts: Vec<String> = Vec::new();
            if term_disabled {
                guard_parts.push("false".into());
            }
            let decls = self.encode_term(term.as_ref(), ty, action);
            guard_parts.extend(decls.guard);

            let trans_id = self.add_transition(name, guard_parts.join(" andalso "));

            // 目标 token 弧：读 = 自环；消费 = 消耗并归档。
            self.arc(
                &PlaceKey::Type(ty.clone()),
                &trans_id,
                "x".into(),
                Direction::PtoT,
            );
            if action == Action::Read {
                self.arc(
                    &PlaceKey::Type(ty.clone()),
                    &trans_id,
                    "x".into(),
                    Direction::TtoP,
                );
            } else {
                let expr = format!("(x, \"{}\", pact_now())", escape(&participant.0));
                self.arc(&PlaceKey::Archive, &trans_id, expr, Direction::TtoP);
                if complements.contains(ty) {
                    self.arc(
                        &PlaceKey::Complement(ty.clone()),
                        &trans_id,
                        "1`0".into(),
                        Direction::TtoP,
                    );
                }
            }

            // 条件弧。
            for (key, dir, expr) in decls.arcs {
                self.arc(&key, &trans_id, expr, dir);
            }

            self.max_witnesses = self.max_witnesses.max(decls.witnesses);
            if term_disabled {
                self.stats.disabled_transitions += 1;
            } else {
                match action {
                    Action::Read => self.stats.read_transitions += 1,
                    Action::Consume => self.stats.consume_transitions += 1,
                    Action::Produce => {}
                }
            }
        }
    }

    /// 把一个 DNF 合取项编码为条件弧 + guard。
    fn encode_term(
        &mut self,
        term: Option<&Term>,
        target_ty: &TypeName,
        action: Action,
    ) -> TermEncoding {
        let mut enc = TermEncoding::default();
        let Some(term) = term else {
            return enc;
        };

        // 目标类型在 Counter 中的 token 自身也计入 count。
        let target_counts = matches!(action, Action::Read | Action::Consume);
        let mut witness = 0usize;

        for (ty, req) in &term.positive {
            let target_is_ty = target_counts && ty == target_ty;
            // via 需要一个见证 token；若见证类型正是目标类型，目标 token 自身
            // 不可能出现在自己的 depends_on 中，故至少需要目标 + 见证两个 token。
            let witness_floor = if req.via && target_is_ty {
                2
            } else if req.via || req.exists {
                1
            } else {
                0
            };
            let total_needed = req.need.max(witness_floor);
            let extra = if target_is_ty {
                total_needed.saturating_sub(1)
            } else {
                total_needed
            };
            let mut vars = Vec::new();
            for _ in 0..extra {
                witness += 1;
                let var = format!("y{witness}");
                enc.arcs
                    .push((PlaceKey::Type(ty.clone()), Direction::PtoT, var.clone()));
                enc.arcs
                    .push((PlaceKey::Type(ty.clone()), Direction::TtoP, var.clone()));
                vars.push(var);
            }
            if req.via {
                if let Some(first) = vars.first() {
                    enc.guard
                        .push(format!("List.exists (fn z => z = #1 {first}) (#4 x)"));
                }
            }
            // 同一类型内的见证变量两两不同（count 语义）。
            for i in 0..vars.len() {
                for j in (i + 1)..vars.len() {
                    enc.guard.push(format!("#1 {} <> #1 {}", vars[i], vars[j]));
                }
            }
            enc.witnesses = enc.witnesses.max(witness);
        }

        for (ty, k) in &term.at_most {
            let bound = self.bounds.get(ty).copied().unwrap_or(0);
            let need = bound.saturating_sub(*k);
            if need == 0 {
                continue;
            }
            let expr = format!("{need}`0");
            enc.arcs.push((
                PlaceKey::Complement(ty.clone()),
                Direction::PtoT,
                expr.clone(),
            ));
            enc.arcs
                .push((PlaceKey::Complement(ty.clone()), Direction::TtoP, expr));
        }
        enc
    }

    fn maintain_complement_on_output(
        &mut self,
        ty: &TypeName,
        trans_id: &str,
        complements: &BTreeSet<TypeName>,
    ) {
        if complements.contains(ty) {
            self.arc(
                &PlaceKey::Complement(ty.clone()),
                trans_id,
                "1`0".into(),
                Direction::PtoT,
            );
        }
    }

    fn finish(mut self) -> CpnModel {
        // 统计实例名后写 XML（stats 的 instance 字段与模型一致）。
        self.stats.instance = self.instance.clone();
        let xml = xml::render(&self);
        CpnModel {
            xml,
            stats: self.stats,
        }
    }
}

#[derive(Default)]
struct TermEncoding {
    arcs: Vec<(PlaceKey, Direction, String)>,
    guard: Vec<String>,
    witnesses: usize,
}

/// XML 文本转义。
fn escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&apos;")
        .replace('"', "&quot;")
}
