//! PACT 事件溯源存储（设计 §7）：JSONL 真相源 + 快照 + turso 派生索引。
//!
//! 依赖方向（design.md §1）：store → core（不依赖 parser——
//! 声明解析留在引擎层，重放经 `apply_event` 完成纯状态部分）。
//!
//! 事件/快照/实例键的类型定义在纯模型层 `pact-store-model`（无 IO，可与 WASM 共用）；
//! 本 crate 只负责落盘、索引、锁与恢复，并原样 re-export 模型类型以保持既有 API。
//!
//! 日志前缀压缩与 schema 版本化见设计 §7.5：`Store::compact` 以快照为提交点
//! 原子截断前缀，事件/快照的 `version` 兼容契约由 `EVENT_SCHEMA_VERSION` 统一承载。

mod audit;
mod error;
mod index;
mod log;
mod snapshot;
mod store;

pub use audit::{ReadAuditKind, ReadAuditRecord, ViewAuditType, ViewName};
pub use error::{validate_domain_name, validate_instance_name, StoreError};
pub use index::{IndexStore, TokenStatus, TursoIndexStore};
pub use log::EventLog;
pub use pact_store_model::{
    apply_event, to_wire_json, ApplyOutcome, Event, InstanceKey, ModelError, Snapshot, WireEvent,
    EVENT_SCHEMA_VERSION,
};
pub use store::{ActivePointer, CompactionResult, RecoveredInstance, Store};
