//! D35：MCP 工具层（daemon 内建 `/mcp` 与 stdio `pact-mcp` 共用）。
//!
//! 21 个 `pact_` 工具经 [`crate::client`] 接入 daemon；错误以 §6.3 结构化 JSON 返回。
//! 域双键（D27）：域级工具均接受可选 `instance`（缺省活动实例，无指针时为 `default`）。

use crate::client::{Client, ClientError};
use crate::dto::ErrorBody;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CustomNotification, ServerNotification};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{tool, tool_handler, tool_router, Peer, RoleServer, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// §6.3 错误体 → 工具错误字符串（Err → CallToolResult.is_error = true）。
fn to_err(e: ClientError) -> String {
    let detail = match &e {
        ClientError::Api(d) => d.clone(),
        ClientError::Io(err) => crate::dto::ErrorDetail {
            code: "Io".to_owned(),
            detail: serde_json::Value::Null,
            message: err.to_string(),
        },
        ClientError::Http(err) => crate::dto::ErrorDetail {
            code: "Transport".to_owned(),
            detail: serde_json::Value::Null,
            message: err.to_string(),
        },
        ClientError::Parse(err) => crate::dto::ErrorDetail {
            code: "Parse".to_owned(),
            detail: serde_json::Value::Null,
            message: err.to_string(),
        },
        ClientError::UnexpectedStatus(s) => crate::dto::ErrorDetail {
            code: "UnexpectedStatus".to_owned(),
            detail: serde_json::json!({ "status": s.as_u16() }),
            message: e.to_string(),
        },
        ClientError::Config(msg) => crate::dto::ErrorDetail {
            code: "Config".to_owned(),
            detail: serde_json::Value::Null,
            message: msg.clone(),
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
struct HandoffIn {
    /// 域名。
    domain: String,
    /// 目标实例使用的新声明原文。
    declaration: String,
    /// 源实例（交接后冻结只读）。
    source_instance: String,
    /// 目标实例（必须不存在）。
    target_instance: String,
    /// 交接后切换活动实例指针（缺省 true）。
    #[serde(default = "default_true")]
    activate: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ActivateIn {
    /// 域名。
    domain: String,
    /// 活动实例名（必须已加载）。
    instance: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ActiveIn {
    /// 域名。
    domain: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FreezeIn {
    /// 域名。
    domain: String,
    /// 实例名。
    instance: String,
    /// 解除冻结（缺省 false = 冻结）。
    #[serde(default)]
    unfreeze: bool,
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

#[derive(Debug, Deserialize, JsonSchema)]
struct CompactIn {
    domain: String,
    /// 目标实例名（缺省 default）。
    #[serde(default)]
    instance: Option<String>,
    /// 保留最近的事件条数（缺省 0 = 丢弃全部已入快照的前缀）。
    #[serde(default)]
    keep_last: u64,
}

// ============ MCP server ============

#[derive(Clone)]
pub struct PactMcp {
    client: Arc<Client>,
    tool_router: ToolRouter<Self>,
}

impl PactMcp {
    pub fn new(client: Arc<Client>) -> Self {
        Self {
            client,
            tool_router: Self::tool_router(),
        }
    }

    /// 每个工具调用的客户端：本地后端按请求注入已验证参与者（不转发原始 JWT）；
    /// 远端后端沿用基客户端配置的 Bearer / 签名身份。
    fn client_for(&self, ctx: &RequestContext<RoleServer>) -> Arc<Client> {
        if !self.client.is_local() {
            return self.client.clone();
        }
        let principal = ctx
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| {
                parts
                    .headers
                    .get(crate::sign::HEADER_VERIFIED_PRINCIPAL)
                    .and_then(|v| v.to_str().ok())
            })
            .map(ToOwned::to_owned);
        Arc::new((*self.client).clone().with_verified_principal(principal))
    }
}

/// D35：`/mcp` 的 Streamable HTTP 服务（daemon 内建）。
pub fn http_service(
    client: Arc<Client>,
    allowed_hosts: Vec<String>,
) -> StreamableHttpService<PactMcp, LocalSessionManager> {
    let mut config = StreamableHttpServerConfig::default().with_legacy_session_mode(true);
    if !allowed_hosts.is_empty() {
        config = config.with_allowed_hosts(allowed_hosts);
    }
    StreamableHttpService::new(
        move || Ok(PactMcp::new(client.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    )
}

#[tool_router]
impl PactMcp {
    #[tool(
        name = "pact_domain_validate",
        description = "校验域声明文本良构，返回结构概要（不加载）"
    )]
    async fn domain_validate(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<DeclarationIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(&client.validate(&in_.declaration).await.map_err(to_err)?)
    }

    #[tool(
        name = "pact_domain_load",
        description = "加载/替换命名域（声明 + 实例化；缺省 default 实例；非空实例需 force）"
    )]
    async fn domain_load(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<LoadIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(
            &client
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
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<InstantiateIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(
            &client
                .instantiate(&in_.domain, &in_.instance)
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_list_domains",
        description = "已加载域清单（定义键 + 实例）"
    )]
    async fn list_domains(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(&client.list_domains().await.map_err(to_err)?)
    }

    #[tool(
        name = "pact_analyze",
        description = "五项静态分析（结构/可达性/参与者/类型/报告）"
    )]
    async fn analyze(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<DomainIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(&client.analyze_domain(&in_.domain).await.map_err(to_err)?)
    }

    #[tool(name = "pact_compose", description = "组合多片段声明并加载为命名域")]
    async fn compose(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<ComposeIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(
            &client
                .compose(&in_.name, in_.fragments)
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_seed",
        description = "Rule-S：种子注入（仅 SeedAuthority 内参与者对源类型）"
    )]
    async fn seed(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<SeedIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(
            &client
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
    async fn produce(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<ProduceIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(
            &client
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
    async fn read(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<TokenIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(
            &client
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
    async fn consume(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<ConsumeIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(
            &client
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
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<TypeFilterIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(
            &client
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
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<TypeFilterIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(
            &client
                .archive(&in_.domain, in_.instance.as_deref(), in_.ty.as_deref())
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_my_gates",
        description = "参与者的全部 gate 子句（含条件），agent 自省"
    )]
    async fn my_gates(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<ParticipantIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(
            &client
                .gates(&in_.domain, in_.instance.as_deref(), &in_.participant)
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_enabled",
        description = "Rule-W 轮询：当前可满足的 Rule-P 组合"
    )]
    async fn enabled(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<ParticipantIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(
            &client
                .enabled(&in_.domain, in_.instance.as_deref(), &in_.participant)
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(name = "pact_status", description = "Counter 概要 + terminated 判定")]
    async fn status(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<DomainIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(
            &client
                .status(&in_.domain, in_.instance.as_deref())
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_compact",
        description = "日志前缀压缩：写全量快照后截断已覆盖的事件前缀（keep_last 保留尾部条数）"
    )]
    async fn compact(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<CompactIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(
            &client
                .compact(&in_.domain, in_.instance.as_deref(), in_.keep_last)
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_domain_handoff",
        description = "状态交接：冻结源实例，以新声明创建目标并继承 Counter/Archive/幂等表，缺省切换活动实例"
    )]
    async fn domain_handoff(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<HandoffIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(
            &client
                .handoff(
                    &in_.domain,
                    &in_.source_instance,
                    &in_.target_instance,
                    &in_.declaration,
                    in_.activate,
                )
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_domain_activate",
        description = "切换域的活动实例指针（短路由 /domains/{d}/... 解析目标）"
    )]
    async fn domain_activate(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<ActivateIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(
            &client
                .activate(&in_.domain, &in_.instance)
                .await
                .map_err(to_err)?,
        )
    }

    #[tool(
        name = "pact_domain_active",
        description = "查询域的活动实例（无指针 → default）"
    )]
    async fn domain_active(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<ActiveIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        to_ok(&client.active_instance(&in_.domain).await.map_err(to_err)?)
    }

    #[tool(
        name = "pact_domain_freeze",
        description = "冻结/解冻实例：冻结后拒绝新的 Seed/Produce/Consume（读与幂等重放仍可用）"
    )]
    async fn domain_freeze(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(in_): Parameters<FreezeIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        client
            .set_frozen(&in_.domain, &in_.instance, !in_.unfreeze)
            .await
            .map_err(to_err)?;
        Ok(serde_json::json!({
            "domain": in_.domain,
            "instance": in_.instance,
            "frozen": !in_.unfreeze
        })
        .to_string())
    }

    #[tool(
        name = "pact_subscribe",
        description = "注册 Counter 变更通知（Rule-W 唤醒）；事件经 notifications/pact/event 推送"
    )]
    async fn subscribe(
        &self,
        ctx: RequestContext<RoleServer>,
        peer: Peer<RoleServer>,
        Parameters(in_): Parameters<DomainIn>,
    ) -> Result<String, String> {
        let client = self.client_for(&ctx);
        let mut stream = client
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
