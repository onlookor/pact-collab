//! PACT MCP server（阶段 4/10，§9.1）：rmcp stdio，16 个 `pact_` 工具，
//! 经 [`pact_http::client`] 接入 daemon；错误以 §6.3 结构化 JSON 返回。
//! 域双键（D27）：域级工具均接受可选 `instance`（缺省 `default`）。

use clap::Parser;
use pact_http::client::{Client, ClientError};
use pact_http::dto::ErrorBody;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CustomNotification, ServerNotification};
use rmcp::{tool, tool_handler, tool_router, Peer, RoleServer, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// §6.3 错误体 → 工具错误字符串（Err → CallToolResult.is_error = true）。
fn to_err(e: ClientError) -> String {
    let detail = match &e {
        ClientError::Api(d) => d.clone(),
        ClientError::Io(err) => pact_http::dto::ErrorDetail {
            code: "Io".to_owned(),
            detail: serde_json::Value::Null,
            message: err.to_string(),
        },
        ClientError::Http(err) => pact_http::dto::ErrorDetail {
            code: "Transport".to_owned(),
            detail: serde_json::Value::Null,
            message: err.to_string(),
        },
        ClientError::Parse(err) => pact_http::dto::ErrorDetail {
            code: "Parse".to_owned(),
            detail: serde_json::Value::Null,
            message: err.to_string(),
        },
        ClientError::UnexpectedStatus(s) => pact_http::dto::ErrorDetail {
            code: "UnexpectedStatus".to_owned(),
            detail: serde_json::json!({ "status": s.as_u16() }),
            message: e.to_string(),
        },
    };
    serde_json::to_string(&ErrorBody { error: detail }).unwrap_or_else(|_| e.to_string())
}

/// 成功响应 → §6 线上格式 JSON 字符串。
fn to_ok<T: Serialize>(v: &T) -> Result<String, String> {
    serde_json::to_string(v).map_err(|e| to_err(ClientError::Parse(e)))
}

// ============ 工具输入（schema 由 schemars 派生） ============

#[derive(Debug, Deserialize, JsonSchema)]
struct DeclarationIn {
    /// 域声明文本（.pact 文件内容）。
    declaration: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LoadIn {
    /// 命名域名（幂等：同名替换）。
    name: String,
    /// 域声明文本。
    declaration: String,
    /// 目标实例名（缺省 default）。
    #[serde(default)]
    instance: Option<String>,
    /// 目标实例非空时允许替换（缺省 false → DomainBusy 冲突）。
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct InstantiateIn {
    /// 已登记的域名。
    domain: String,
    /// 新实例名。
    instance: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DomainIn {
    /// 域名。
    domain: String,
    /// 目标实例名（缺省 default）。
    #[serde(default)]
    instance: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ComposeIn {
    /// 组合结果域名。
    name: String,
    /// 片段声明文本列表。
    fragments: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SeedIn {
    domain: String,
    /// 目标实例名（缺省 default）。
    #[serde(default)]
    instance: Option<String>,
    participant: String,
    /// 源类型名（线上字段名 `type`）。
    #[serde(rename = "type")]
    #[schemars(rename = "type")]
    ty: String,
    /// 可选：关联外部事物引用。
    #[serde(default)]
    thing_ref: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProduceIn {
    domain: String,
    /// 目标实例名（缺省 default）。
    #[serde(default)]
    instance: Option<String>,
    participant: String,
    #[serde(rename = "type")]
    #[schemars(rename = "type")]
    ty: String,
    /// 前驱 token id 列表（须与 Precond(type) 匹配，互异）。
    #[serde(default)]
    predecessors: Vec<String>,
    #[serde(default)]
    thing_ref: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TokenIn {
    domain: String,
    /// 目标实例名（缺省 default）。
    #[serde(default)]
    instance: Option<String>,
    participant: String,
    token_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ConsumeIn {
    domain: String,
    /// 目标实例名（缺省 default）。
    #[serde(default)]
    instance: Option<String>,
    participant: String,
    token_id: String,
    /// 可选幂等键：同实例内重复请求返回首次结果，不重复消费。
    #[serde(default)]
    request_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TypeFilterIn {
    domain: String,
    /// 目标实例名（缺省 default）。
    #[serde(default)]
    instance: Option<String>,
    /// 可选类型过滤。
    #[serde(rename = "type", default)]
    #[schemars(rename = "type")]
    ty: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ParticipantIn {
    domain: String,
    /// 目标实例名（缺省 default）。
    #[serde(default)]
    instance: Option<String>,
    participant: String,
}

// ============ MCP server ============

#[derive(Clone)]
struct PactMcp {
    client: Arc<Client>,
    tool_router: ToolRouter<Self>,
}

impl PactMcp {
    fn new(client: Arc<Client>) -> Self {
        Self {
            client,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl PactMcp {
    #[tool(
        name = "pact_domain_validate",
        description = "校验域声明文本良构，返回结构概要（不加载）"
    )]
    async fn domain_validate(
        &self,
        Parameters(in_): Parameters<DeclarationIn>,
    ) -> Result<String, String> {
        to_ok(
            &self
                .client
                .validate(&in_.declaration)
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_domain_load",
        description = "加载/替换命名域（声明 + 实例化；缺省 default 实例；非空实例需 force）"
    )]
    async fn domain_load(&self, Parameters(in_): Parameters<LoadIn>) -> Result<String, String> {
        to_ok(
            &self
                .client
                .load_domain(
                    &in_.name,
                    &in_.declaration,
                    in_.instance.as_deref(),
                    in_.force,
                )
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_domain_instantiate",
        description = "在已登记声明上创建额外实例（不改变声明）"
    )]
    async fn domain_instantiate(
        &self,
        Parameters(in_): Parameters<InstantiateIn>,
    ) -> Result<String, String> {
        to_ok(
            &self
                .client
                .instantiate(&in_.domain, &in_.instance)
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_list_domains",
        description = "已加载域清单（定义键 + 实例）"
    )]
    async fn list_domains(&self) -> Result<String, String> {
        to_ok(&self.client.list_domains().await.map_err(to_err)?)
    }

    #[tool(
        name = "pact_analyze",
        description = "五项静态分析（结构/可达性/参与者/类型/报告）"
    )]
    async fn analyze(&self, Parameters(in_): Parameters<DomainIn>) -> Result<String, String> {
        to_ok(
            &self
                .client
                .analyze_domain(&in_.domain)
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(name = "pact_compose", description = "组合多片段声明并加载为命名域")]
    async fn compose(&self, Parameters(in_): Parameters<ComposeIn>) -> Result<String, String> {
        to_ok(
            &self
                .client
                .compose(&in_.name, in_.fragments)
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_seed",
        description = "Rule-S：种子注入（仅 SeedAuthority 内参与者对源类型）"
    )]
    async fn seed(&self, Parameters(in_): Parameters<SeedIn>) -> Result<String, String> {
        to_ok(
            &self
                .client
                .seed(
                    &in_.domain,
                    in_.instance.as_deref(),
                    &in_.participant,
                    &in_.ty,
                    in_.thing_ref.as_deref(),
                )
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_produce",
        description = "Rule-P：显式传前驱 token id 列表产出新 token"
    )]
    async fn produce(&self, Parameters(in_): Parameters<ProduceIn>) -> Result<String, String> {
        to_ok(
            &self
                .client
                .produce(
                    &in_.domain,
                    in_.instance.as_deref(),
                    &in_.participant,
                    &in_.ty,
                    in_.predecessors,
                    in_.thing_ref.as_deref(),
                )
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_read",
        description = "Rule-R：读标记（返回 token 全量数据，不改变 Counter）"
    )]
    async fn read(&self, Parameters(in_): Parameters<TokenIn>) -> Result<String, String> {
        to_ok(
            &self
                .client
                .read_token(
                    &in_.domain,
                    in_.instance.as_deref(),
                    &in_.participant,
                    &in_.token_id,
                )
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_consume",
        description = "Rule-E：消费 token（移出 Counter，归档）；可选 request_id 幂等去重"
    )]
    async fn consume(&self, Parameters(in_): Parameters<ConsumeIn>) -> Result<String, String> {
        to_ok(
            &self
                .client
                .consume(
                    &in_.domain,
                    in_.instance.as_deref(),
                    &in_.participant,
                    &in_.token_id,
                    in_.request_id.as_deref(),
                )
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_counter_query",
        description = "Counter 状态查询（可按类型过滤）"
    )]
    async fn counter_query(
        &self,
        Parameters(in_): Parameters<TypeFilterIn>,
    ) -> Result<String, String> {
        to_ok(
            &self
                .client
                .counter(&in_.domain, in_.instance.as_deref(), in_.ty.as_deref())
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_archive_query",
        description = "Archive 状态查询（可按类型过滤）"
    )]
    async fn archive_query(
        &self,
        Parameters(in_): Parameters<TypeFilterIn>,
    ) -> Result<String, String> {
        to_ok(
            &self
                .client
                .archive(&in_.domain, in_.instance.as_deref(), in_.ty.as_deref())
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_my_gates",
        description = "参与者的全部 gate 子句（含条件），agent 自省"
    )]
    async fn my_gates(&self, Parameters(in_): Parameters<ParticipantIn>) -> Result<String, String> {
        to_ok(
            &self
                .client
                .gates(&in_.domain, in_.instance.as_deref(), &in_.participant)
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_enabled",
        description = "Rule-W 轮询：当前可满足的 Rule-P 组合"
    )]
    async fn enabled(&self, Parameters(in_): Parameters<ParticipantIn>) -> Result<String, String> {
        to_ok(
            &self
                .client
                .enabled(&in_.domain, in_.instance.as_deref(), &in_.participant)
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(name = "pact_status", description = "Counter 概要 + terminated 判定")]
    async fn status(&self, Parameters(in_): Parameters<DomainIn>) -> Result<String, String> {
        to_ok(
            &self
                .client
                .status(&in_.domain, in_.instance.as_deref())
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_subscribe",
        description = "注册 Counter 变更通知（Rule-W 唤醒）；事件经 notifications/pact/event 推送"
    )]
    async fn subscribe(
        &self,
        peer: Peer<RoleServer>,
        Parameters(in_): Parameters<DomainIn>,
    ) -> Result<String, String> {
        let mut stream = self
            .client
            .events(&in_.domain, in_.instance.as_deref())
            .await
            .map_err(to_err)?;
        tokio::spawn(async move {
            while let Ok(Some(n)) = stream.next_event().await {
                let params = serde_json::to_value(&n).unwrap_or_default();
                let res = peer
                    .send_notification(ServerNotification::CustomNotification(
                        CustomNotification::new("notifications/pact/event", Some(params)),
                    ))
                    .await;
                if res.is_err() {
                    break; // 客户端断开，停止转发
                }
            }
        });
        Ok(serde_json::json!({ "subscribed": true, "domain": in_.domain }).to_string())
    }
}

#[tool_handler(
    router = self.tool_router,
    name = "pact-mcp",
    instructions = "PACT 范式运行时：前置条件治理的自治计数器令牌。先 pact_domain_load 加载域，经 pact_seed / pact_produce / pact_read / pact_consume 推进流程，用 pact_enabled / pact_status / pact_subscribe 观察与唤醒。"
)]
impl ServerHandler for PactMcp {}

// ============ 启动 ============

#[derive(Debug, Parser)]
#[command(name = "pact-mcp", about = "PACT MCP server（stdio）")]
struct Args {
    /// unix socket 路径（缺省 ./pact.sock）。
    #[arg(long)]
    unix: Option<PathBuf>,
    /// TCP 地址（如 127.0.0.1:8787）；提供时优先于 --unix。
    #[arg(long)]
    tcp: Option<String>,
    /// 参与者令牌（§9.4 信任声明模式下可省略）。
    #[arg(long)]
    bearer: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();
    let mut client = match args.tcp {
        Some(addr) => Client::tcp(addr),
        None => Client::unix(args.unix.unwrap_or_else(|| PathBuf::from("./pact.sock"))),
    };
    if let Some(t) = args.bearer {
        client = client.with_bearer(t);
    }
    let server = PactMcp::new(Arc::new(client));
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
