//! PACT 引擎（设计 §8）：tokio actor，单权威串行执行（D6）。
//!
//! 职责边界（引擎只编排不判定，D14）：
//! - 命令串行化：mpsc 通道 + actor 任务，检查与应用在同一临界区内原子完成；
//! - 多域会话（D7）+ 域双键（D27）：一个引擎实例承载多个定义键与其实例；
//! - 依赖注入：Clock（D8）与 IdGen（D10）由构造方注入，
//!   pact-core 保持纯函数语义；
//! - 通知总线（D4）：每域实例 broadcast 通道，Rule-W 唤醒；
//! - 持久化编排：调用 pact-store 完成事件落盘、快照与恢复。
//!
//! 模块：
//! - [`engine`]：actor、命令、`Engine` 句柄（公共 API）；
//! - [`clock`] / [`idgen`]：时间与 id 抽象（生产/测试双实现）；
//! - [`notify`]：状态变更通知（线上格式 §6.4）；
//! - [`views`]：查询视图（线上格式 §6.5，三接口共享）。

pub mod clock;
pub mod engine;
pub mod idgen;
pub mod notify;
pub mod views;

pub use clock::{iso8601_utc, Clock, LogicalClock, SystemClock};
pub use engine::{Command, Engine, EngineConfig, EngineError, DEFAULT_INSTANCE};
pub use idgen::{IdGen, SequentialIdGen, UlidGen};
pub use notify::{Notification, NotificationKind, NotificationSummary};
pub use pact_store::InstanceKey;
pub use views::{
    AnalysisReport, ArchiveView, CounterView, DomainListing, DomainSummary, GateClause, StatusView,
};
