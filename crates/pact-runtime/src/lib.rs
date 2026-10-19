//! PACT 生产运行层（阶段 7，决策 D24）。
//!
//! 引擎保持被动（D5）：本 crate 承载**参与者侧**的自驱行为，不修改 core / store / engine
//! 的语义，也不反向被它们依赖。
//!
//! 三块能力：
//!
//! 1. [`drive`]——可复用的参与者自驱循环：给定参与者名与其门（`my_gates` / `enabled` /
//!    `subscribe`），实现 P1（生产优先）/ P2（汇消费）/ P3（等待）策略，驱动至 Rule-T 终止。
//! 2. [`Orchestrator`]——多 agent 编排：并发拉起 N 个真实 MCP（stdio 子进程）或 REST 参与者。
//! 3. [`SideEffectExecutor`]——外部副作用执行器：订阅 `Consumed`，落地外部动作，失败时以
//!    Rule-S 追加补偿事实（saga），不与外部副作用做原子绑定（FR15 / D23）。
//!
//! 依赖方向：`runtime → {http, engine, core}`（单向）。`pact-mcp` 为 bin-only，本 crate 不依赖
//! 其 lib，仅按需 spawn 其可执行文件。

pub mod client;
pub mod driver;
pub mod error;
pub mod executor;
pub mod mcp;
pub mod orchestrator;
pub mod rest;

pub use client::{Endpoint, ParticipantClient, WakeStream};
pub use driver::{drive, AgentReport, Bootstrap, DriverConfig, DEFAULT_DEADLINE, DEFAULT_POLL};
pub use error::RuntimeError;
pub use executor::{
    Compensation, ExecutorConfig, ExecutorReport, SideEffectExecutor, SideEffectHandler,
    StopSignal, DEFAULT_EXECUTOR_POLL,
};
pub use mcp::McpClient;
pub use orchestrator::{AgentSpec, Orchestrator, Transport};
pub use rest::RestClient;
