//! PACT 范式核心语义（论文 `docs/pactvfinal.md`）。
//!
//! 纯函数实现，硬性纯度约束（design.md §2.1）：
//! 1. 无 IO：不依赖任何 IO/async crate，不使用 std::fs / std::net / std::process / 环境变量；
//! 2. 无日志：不使用 tracing/log 宏（可观测性经返回值传递，由引擎层落日志）；
//! 3. 无时钟：不读取系统时间，时间戳一律由调用方传入；
//! 4. 无随机：不生成 TokenId，id 一律由调用方传入；
//! 5. 无全局可变状态；
//! 6. 依赖白名单：serde（仅 derive）为唯一非 std 依赖；
//! 7. 规则函数只读状态：签名层面强制"检查不改状态"。

pub mod domain;
pub mod effect;
pub mod gate;
pub mod rules;
pub mod types;
pub mod wait;

#[cfg(test)]
mod tests;

pub use domain::{is_source_type, validate_domain, Domain, DomainError};
pub use effect::Effect;
pub use gate::{gate_holds, CountOp, GateExpr, GateFailure};
pub use rules::{precond_of, rule_e, rule_p, rule_r, rule_s, RuleError};
pub use types::{
    Action, ArchiveEntry, CounterState, GuardedAction, ParticipantName, ThingRef, Timestamp, Token,
    TokenId, TypeName,
};
pub use wait::{enabled_productions, termination_holds, ProductionOption};
