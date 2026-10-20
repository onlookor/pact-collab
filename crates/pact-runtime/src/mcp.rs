//! MCP（stdio）参与者客户端：拉起 `pact-mcp` 子进程，经 15 个 `pact_*` 工具映射引擎命令。
//!
//! 与 `pact-mcp` 的 REST 接入路径（D19）等价——MCP 只是 REST 的形态转换。本模块的客户端
//! 侧用法与 `crates/pact-mcp/tests/unattended_e2e.rs` 同构（子进程 + `serve` 握手 + 工具调用 +
//! `notifications/pact/event` 自定义通知）。

use crate::client::{Endpoint, ParticipantClient, WakeStream};
use crate::error::RuntimeError;
use async_trait::async_trait;
use pact_core::{ArchiveEntry, Token};
use pact_engine::notify::Notification;
use pact_engine::views::{
    ArchiveView, CounterView, DomainListing, DomainSummary, GateClause, StatusView,
};
use pact_http::dto::{
    DomainsResp, EnabledOption, EnabledResp, EntryResp, ErrorBody, GatesResp, TokenResp,
};
use rmcp::model::{CallToolRequestParams, ContentBlock, CustomNotification};
use rmcp::service::{NotificationContext, RunningService};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::path::Path;
use tokio::sync::{mpsc, Mutex};

/// MCP 客户端通知处理器：把 `notifications/pact/event` 载荷转成 [`Notification`] 送入唤醒通道。
#[derive(Clone)]
struct EventNotifier {
    tx: mpsc::UnboundedSender<Notification>,
}

impl ClientHandler for EventNotifier {
    async fn on_custom_notification(
        &self,
        notification: CustomNotification,
        _context: NotificationContext<RoleClient>,
    ) {
        if notification.method == "notifications/pact/event" {
            if let Some(params) = notification.params {
                if let Ok(n) = serde_json::from_value::<Notification>(params) {
                    let _ = self.tx.send(n);
                }
            }
        }
    }
}

/// 经 MCP（stdio 子进程）接入 daemon 的参与者客户端。
pub struct McpClient {
    svc: RunningService<RoleClient, EventNotifier>,
    wake: Mutex<Option<mpsc::UnboundedReceiver<Notification>>>,
    domain: String,
    /// 实例键；`None` = `default` 实例（D27）。
    instance: Option<String>,
}

impl McpClient {
    /// 拉起 `bin` 子进程并完成 MCP 握手（作用于 `default` 实例）。
    pub async fn spawn(
        bin: impl AsRef<Path>,
        endpoint: &Endpoint,
        bearer: Option<&str>,
        domain: impl Into<String>,
    ) -> Result<Self, RuntimeError> {
        let (tx, rx) = mpsc::unbounded_channel();
        let cmd = tokio::process::Command::new(bin.as_ref()).configure(|c| {
            for a in endpoint.args() {
                c.arg(a);
            }
            if let Some(t) = bearer {
                c.arg("--bearer").arg(t);
            }
        });
        let (transport, _stderr) = TokioChildProcess::builder(cmd)
            .spawn()
            .map_err(|e| RuntimeError::Transport(format!("启动 MCP 子进程失败：{e}")))?;
        let svc = EventNotifier { tx }
            .serve(transport)
            .await
            .map_err(|e| RuntimeError::Protocol(format!("MCP 握手失败：{e}")))?;
        Ok(Self {
            svc,
            wake: Mutex::new(Some(rx)),
            domain: domain.into(),
            instance: None,
        })
    }

    /// 绑定实例键（缺省 `default` 实例，D27）。
    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    /// 调用工具并把 §6 线上格式 JSON 解析为 `T`；工具报错时按 §6.3 结构化体映射为
    /// [`RuntimeError::Api`]。
    async fn call<T: DeserializeOwned>(&self, name: &str, args: Value) -> Result<T, RuntimeError> {
        let arguments: serde_json::Map<String, Value> = serde_json::from_value(args)
            .map_err(|e| RuntimeError::Protocol(format!("工具 {name} 参数构造失败：{e}")))?;
        let params = CallToolRequestParams::new(name.to_owned()).with_arguments(arguments);
        let res = self
            .svc
            .call_tool(params)
            .await
            .map_err(|e| RuntimeError::Transport(format!("调用 {name} 失败：{e}")))?;
        let text = match res.content.first() {
            Some(ContentBlock::Text(t)) => t.text.clone(),
            _ => {
                return Err(RuntimeError::Protocol(format!(
                    "工具 {name} 返回非文本内容"
                )))
            }
        };
        if res.is_error.unwrap_or(false) {
            if let Ok(body) = serde_json::from_str::<ErrorBody>(&text) {
                return Err(RuntimeError::Api {
                    code: body.error.code,
                    message: body.error.message,
                });
            }
            return Err(RuntimeError::Api {
                code: "ToolError".to_owned(),
                message: text,
            });
        }
        serde_json::from_str(&text)
            .map_err(|e| RuntimeError::Protocol(format!("工具 {name} 输出非法 JSON：{e}")))
    }
}

#[async_trait]
impl ParticipantClient for McpClient {
    fn domain(&self) -> &str {
        &self.domain
    }

    fn instance(&self) -> Option<&str> {
        self.instance.as_deref()
    }

    async fn list_domains(&self) -> Result<Vec<DomainListing>, RuntimeError> {
        let r: DomainsResp = self.call("pact_list_domains", json!({})).await?;
        Ok(r.domains)
    }

    async fn load_domain(&self, declaration: &str) -> Result<(), RuntimeError> {
        let _: DomainSummary = self
            .call(
                "pact_domain_load",
                json!({
                    "name": self.domain,
                    "declaration": declaration,
                    "instance": self.instance,
                }),
            )
            .await?;
        Ok(())
    }

    async fn seed(
        &self,
        participant: &str,
        ty: &str,
        thing_ref: Option<&str>,
    ) -> Result<Token, RuntimeError> {
        let r: TokenResp = self
            .call(
                "pact_seed",
                json!({
                    "domain": self.domain,
                    "instance": self.instance,
                    "participant": participant,
                    "type": ty,
                    "thing_ref": thing_ref,
                }),
            )
            .await?;
        Ok(r.token)
    }

    async fn produce(
        &self,
        participant: &str,
        ty: &str,
        predecessors: &[String],
    ) -> Result<Token, RuntimeError> {
        let r: TokenResp = self
            .call(
                "pact_produce",
                json!({
                    "domain": self.domain,
                    "instance": self.instance,
                    "participant": participant,
                    "type": ty,
                    "predecessors": predecessors,
                }),
            )
            .await?;
        Ok(r.token)
    }

    async fn read(&self, participant: &str, token_id: &str) -> Result<Token, RuntimeError> {
        let r: TokenResp = self
            .call(
                "pact_read",
                json!({
                    "domain": self.domain,
                    "instance": self.instance,
                    "participant": participant,
                    "token_id": token_id,
                }),
            )
            .await?;
        Ok(r.token)
    }

    async fn consume(
        &self,
        participant: &str,
        token_id: &str,
        request_id: Option<&str>,
    ) -> Result<ArchiveEntry, RuntimeError> {
        let r: EntryResp = self
            .call(
                "pact_consume",
                json!({
                    "domain": self.domain,
                    "instance": self.instance,
                    "participant": participant,
                    "token_id": token_id,
                    "request_id": request_id,
                }),
            )
            .await?;
        Ok(r.entry)
    }

    async fn my_gates(&self, participant: &str) -> Result<Vec<GateClause>, RuntimeError> {
        let r: GatesResp = self
            .call(
                "pact_my_gates",
                json!({
                    "domain": self.domain,
                    "instance": self.instance,
                    "participant": participant,
                }),
            )
            .await?;
        Ok(r.gates)
    }

    async fn enabled(&self, participant: &str) -> Result<Vec<EnabledOption>, RuntimeError> {
        let r: EnabledResp = self
            .call(
                "pact_enabled",
                json!({
                    "domain": self.domain,
                    "instance": self.instance,
                    "participant": participant,
                }),
            )
            .await?;
        Ok(r.options)
    }

    async fn counter(&self) -> Result<CounterView, RuntimeError> {
        self.call(
            "pact_counter_query",
            json!({ "domain": self.domain, "instance": self.instance }),
        )
        .await
    }

    async fn archive(&self) -> Result<Vec<ArchiveEntry>, RuntimeError> {
        let r: ArchiveView = self
            .call(
                "pact_archive_query",
                json!({ "domain": self.domain, "instance": self.instance }),
            )
            .await?;
        Ok(r.entries)
    }

    async fn status(&self) -> Result<StatusView, RuntimeError> {
        self.call(
            "pact_status",
            json!({ "domain": self.domain, "instance": self.instance }),
        )
        .await
    }

    async fn subscribe(&self) -> Result<WakeStream, RuntimeError> {
        let _: Value = self
            .call(
                "pact_subscribe",
                json!({ "domain": self.domain, "instance": self.instance }),
            )
            .await?;
        let rx =
            self.wake.lock().await.take().ok_or_else(|| {
                RuntimeError::Protocol("重复订阅：MCP 唤醒通道已被取走".to_owned())
            })?;
        Ok(WakeStream::from_receiver(rx))
    }

    async fn close(&mut self) -> Result<(), RuntimeError> {
        let _ = self.svc.close().await;
        Ok(())
    }
}
