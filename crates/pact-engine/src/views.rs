//! 引擎查询视图（线上格式设计 §6.5 / §9；三接口共享）。
//!
//! 类型定义已移至纯 crate `pact-views`（无 tokio/IO），供原生引擎与 WASM 门面共用；
//! 本模块 re-export 以保持 `pact_engine::views::*` 既有路径不变。

pub use pact_views::*;
