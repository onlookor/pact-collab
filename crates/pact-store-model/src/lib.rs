//! PACT 事件溯源**纯模型层**（设计 §7.1/§7.5）。
//!
//! 本 crate 只承载事件、快照、实例键与对应错误，不触碰文件系统、网络、
//! 线程或异步运行时。原生 `pact-store` 负责 JSONL/快照/turso/锁，
//! WASM 门面负责由 JS 宿主提供存储；两者共用这里的类型与 schema 版本，
//! 保证磁盘格式与线上格式只有一份定义。
//!
//! 纯度约束：依赖仅 `pact-core` + `serde` + `serde_json`；无 `unsafe`。

mod error;
mod event;
mod key;
mod snapshot;

pub use error::{validate_domain_name, validate_instance_name, ModelError};
pub use event::{
    apply_event, to_wire_json, ApplyOutcome, Event, WireEvent, EVENT_SCHEMA_VERSION,
    IMPORTED_EVENT_SCHEMA_VERSION, SUPPORTED_EVENT_SCHEMA_VERSION,
};
pub use key::InstanceKey;
pub use snapshot::Snapshot;
