//! 效果模型：状态变更的唯一路径（引擎与测试共用，保证重放与在线行为一致）。

use crate::types::{ArchiveEntry, CounterState, Token};

/// 规则成功执行的效果（论文 §1.4 各规则的结论部分）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Rule-S 效果：新 token 入 Counter。
    Seeded(Token),
    /// Rule-P 效果：新 token 入 Counter（前驱不动）。
    Produced(Token),
    /// Rule-E 效果：目标 token 移出 Counter、入 Archive。
    Consumed(ArchiveEntry),
}

impl Effect {
    /// 产生该效果的 token id（引擎广播通知与持久化用）。
    pub fn token_id(&self) -> &crate::types::TokenId {
        match self {
            Effect::Seeded(t) | Effect::Produced(t) => &t.id,
            Effect::Consumed(e) => &e.token.id,
        }
    }
}

impl CounterState {
    /// 应用规则效果。设计不变量（属性测试锁定）：
    /// - Seeded/Produced 只插入新 token，不触碰任何既有 token；
    /// - Consumed 只移除目标 token 并追加 Archive，无级联。
    pub fn apply(&mut self, effect: Effect) {
        match effect {
            Effect::Seeded(t) | Effect::Produced(t) => {
                self.counter.insert(t.id.clone(), t);
            }
            Effect::Consumed(e) => {
                self.counter.remove(&e.token.id);
                self.archive.push(e);
            }
        }
    }
}
