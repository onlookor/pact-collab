//! PACT HTTP 层：线上格式 DTO、daemon 与类型化 client。
//!
//! 设计依据 `docs/design.md` §6（线上格式）与 §9.2（REST 端点）。

pub mod client;
pub mod config;
pub mod daemon;
pub mod dto;
pub mod mcp;
pub mod middleware;
pub mod oidc;
pub mod server;
pub mod sign;
pub mod tls;
