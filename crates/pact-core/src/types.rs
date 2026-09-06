//! 基础类型：论文 §1.2 六原语及派生概念的数据表示。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// —— newtype 标识符（防混用；serde 对单字段元组结构体透明序列化，即线上直接是字符串）——

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TokenId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TypeName(pub String);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ParticipantName(pub String);

/// 对引擎不透明的外部工件引用（论文 §3.2：URI | Identifier | ∅）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThingRef(pub Option<String>);

impl ThingRef {
    pub fn none() -> Self {
        Self(None)
    }
    pub fn some(s: impl Into<String>) -> Self {
        Self(Some(s.into()))
    }
}

/// ISO-8601 UTC 字符串。格式由调用方（引擎层 Clock）保证，core 不校验。
pub type Timestamp = String;

/// 动作（论文 §3.1 ActionClause）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Read,
    Consume,
    Produce,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Read => "read",
            Action::Consume => "consume",
            Action::Produce => "produce",
        }
    }
}

/// GateHolds 的作用域（论文 §1.4.0：条件子句仅作用于 read/consume）。
/// 类型层面阻止 Produce 误入 gate_holds。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardedAction {
    Read,
    Consume,
}

impl From<GuardedAction> for Action {
    fn from(g: GuardedAction) -> Action {
        match g {
            GuardedAction::Read => Action::Read,
            GuardedAction::Consume => Action::Consume,
        }
    }
}

// —— Token（论文 §3.2）——

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub id: TokenId,
    /// 线上格式字段名为 "type"（design.md §6.2）。
    #[serde(rename = "type")]
    pub ty: TypeName,
    pub thing_ref: ThingRef,
    pub produced_by: ParticipantName,
    /// 直接前驱实例 id，生产时记录（审计溯源用；不约束前驱生命周期，§1.4 Note）。
    pub depends_on: Vec<TokenId>,
    pub produced_at: Timestamp,
}

/// Archive 条目（论文 §3.4）：token 离开 Counter 时归档。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveEntry {
    pub token: Token,
    pub exhausted_by: ParticipantName,
    pub exhausted_at: Timestamp,
}

/// Counter 状态：共享可见多重集 + 不可变 Archive（论文 §1.2、§1.4.1）。
/// TokenId 全局唯一（§1.4.1），故以 BTreeMap 实现，按键序确定迭代顺序。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CounterState {
    pub counter: BTreeMap<TokenId, Token>,
    pub archive: Vec<ArchiveEntry>,
}

impl CounterState {
    pub fn new() -> Self {
        Self::default()
    }
}

// —— Display（错误信息与 CLI 输出用）——

impl std::fmt::Display for TokenId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Display for TypeName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Display for ParticipantName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::fmt::Display for ThingRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(s) => f.write_str(s),
            None => f.write_str("∅"),
        }
    }
}
