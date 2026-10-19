//! 参与者客户端抽象：驱动循环所需的最小操作面，REST 与 MCP 两种实现共享。

use crate::error::RuntimeError;
use async_trait::async_trait;
use pact_core::{ArchiveEntry, Token};
use pact_engine::notify::Notification;
use pact_engine::views::{CounterView, GateClause, StatusView};
use pact_http::dto::EnabledOption;
use std::path::PathBuf;
use tokio::sync::mpsc;

/// daemon 端点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// 本地 unix socket。
    Unix(PathBuf),
    /// TCP 地址（`host:port`）。
    Tcp(String),
}

impl Endpoint {
    /// unix socket 端点。
    pub fn unix(path: impl Into<PathBuf>) -> Self {
        Endpoint::Unix(path.into())
    }

    /// TCP 端点。
    pub fn tcp(addr: impl Into<String>) -> Self {
        Endpoint::Tcp(addr.into())
    }

    /// 追加到 `pact-mcp` 子进程命令行的连接参数（与 CLI/MCP 的 flag 约定一致，D20）。
    pub fn args(&self) -> Vec<String> {
        match self {
            Endpoint::Unix(p) => vec!["--unix".to_owned(), p.display().to_string()],
            Endpoint::Tcp(a) => vec!["--tcp".to_owned(), a.clone()],
        }
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Unix(p) => write!(f, "unix:{}", p.display()),
            Endpoint::Tcp(a) => write!(f, "tcp:{a}"),
        }
    }
}

/// Rule-W 唤醒流：由推送（SSE / MCP 通知）转发的 Counter 变更通知。
///
/// 推送仅为**唤醒信号与提示**；决策始终以 daemon 查询结果为准（D4 轮询兜底）。
pub struct WakeStream {
    rx: mpsc::UnboundedReceiver<Notification>,
}

impl WakeStream {
    pub(crate) fn from_receiver(rx: mpsc::UnboundedReceiver<Notification>) -> Self {
        Self { rx }
    }

    /// 下一条通知；`None` 表示推送通道已关闭（调用方应转为纯轮询兜底）。
    pub async fn recv(&mut self) -> Option<Notification> {
        self.rx.recv().await
    }

    /// 非阻塞取一条已就绪通知。
    pub fn try_recv(&mut self) -> Option<Notification> {
        self.rx.try_recv().ok()
    }
}

/// 参与者面向 daemon 的操作面（驱动循环 / 执行器所需最小集合）。
///
/// 所有方法对**绑定域**操作（构造时指定）；`load_domain` 亦作用于该域。
#[async_trait]
pub trait ParticipantClient: Send + Sync {
    /// 绑定的域名。
    fn domain(&self) -> &str;

    /// 列出 daemon 上已加载的域。
    async fn list_domains(&self) -> Result<Vec<String>, RuntimeError>;

    /// 加载/覆盖绑定域的声明（引导参与者使用）。
    async fn load_domain(&self, declaration: &str) -> Result<(), RuntimeError>;

    /// Rule-S 种子注入。
    async fn seed(
        &self,
        participant: &str,
        ty: &str,
        thing_ref: Option<&str>,
    ) -> Result<Token, RuntimeError>;

    /// Rule-P 生产。
    async fn produce(
        &self,
        participant: &str,
        ty: &str,
        predecessors: &[String],
    ) -> Result<Token, RuntimeError>;

    /// Rule-R 读取。
    async fn read(&self, participant: &str, token_id: &str) -> Result<Token, RuntimeError>;

    /// Rule-E 消费（可选幂等键，D23）。
    async fn consume(
        &self,
        participant: &str,
        token_id: &str,
        request_id: Option<&str>,
    ) -> Result<ArchiveEntry, RuntimeError>;

    /// 参与者能力查询。
    async fn my_gates(&self, participant: &str) -> Result<Vec<GateClause>, RuntimeError>;

    /// Rule-W 可满足性查询（当前可执行的 Rule-P 选项）。
    async fn enabled(&self, participant: &str) -> Result<Vec<EnabledOption>, RuntimeError>;

    /// Counter 快照。
    async fn counter(&self) -> Result<CounterView, RuntimeError>;

    /// Archive 快照。
    async fn archive(&self) -> Result<Vec<ArchiveEntry>, RuntimeError>;

    /// 域状态（Counter 大小 / Archive 大小 / Rule-T 终止）。
    async fn status(&self) -> Result<StatusView, RuntimeError>;

    /// 订阅 Counter 变更推送。
    async fn subscribe(&self) -> Result<WakeStream, RuntimeError>;

    /// 关闭客户端（MCP 关闭会话；REST 默认 no-op）。
    async fn close(&mut self) -> Result<(), RuntimeError> {
        Ok(())
    }
}
