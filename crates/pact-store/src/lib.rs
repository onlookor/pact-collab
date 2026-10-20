//! PACT 事件溯源存储（设计 §7）：JSONL 真相源 + 快照 + turso 派生索引。
//!
//! 依赖方向（design.md §1）：store → core（不依赖 parser——
//! 声明解析留在引擎层，重放经 `apply_event` 完成纯状态部分）。

mod error;
mod event;
mod index;
mod key;
mod log;
mod snapshot;
mod store;

pub use error::{validate_domain_name, validate_instance_name, StoreError};
pub use event::{apply_event, ApplyOutcome, Event};
pub use index::{IndexStore, TokenStatus, TursoIndexStore};
pub use key::InstanceKey;
pub use log::EventLog;
pub use snapshot::Snapshot;
pub use store::{RecoveredInstance, Store};
