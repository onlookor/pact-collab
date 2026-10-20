//! REST daemon（设计 §9.2）：axum 路由 + unix socket / TCP 双监听。
//!
//! 被拒动作返回 4xx + §6.3 结构化错误体；错误码 = RuleError / Problem 变体名。

use crate::config::{acl_allows, acl_allows_domain, AclMap};
use crate::dto::{
    conflicts_detail, problem_detail, rule_error_detail, CompactReq, ComposeReq, ConsumeReq,
    DomainInfo, DomainsResp, EnabledOption, EnabledResp, EntryResp, ErrorBody, ErrorDetail,
    GatesResp, InstantiateReq, LoadDomainReq, ParticipantReq, ProduceReq, ReadAuditResp, SeedReq,
    TokenResp, ValidateReq, ValidateResp,
};
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use futures::StreamExt;
use pact_core::{ParticipantName, ThingRef, TokenId, TypeName};
use pact_engine::views::StatusView;
use pact_engine::{Engine, EngineError, InstanceKey, DEFAULT_INSTANCE};
use pact_store::StoreError;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::path::Path as FsPath;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{watch, Notify};
use tokio_stream::wrappers::BroadcastStream;

// ============ 状态与错误 ============

pub struct ServerState {
    engine: Engine,
    /// 参与者 → 令牌；空表 = 信任声明模式（NFR5）。
    auth_tokens: BTreeMap<String, String>,
    /// 域级授权表（FR17/FR19）；空表 = 不启用 ACL（向后兼容）。
    acl: AclMap,
    /// 启动时刻（`/metrics` 的 uptime 基准）。
    started_at: std::time::Instant,
}

impl ServerState {
    /// 无 ACL（向后兼容；等价于 `with_acl(..., BTreeMap::new())`）。
    pub fn new(engine: Engine, auth_tokens: BTreeMap<String, String>) -> Arc<Self> {
        Self::with_acl(engine, auth_tokens, BTreeMap::new())
    }

    pub fn with_acl(
        engine: Engine,
        auth_tokens: BTreeMap<String, String>,
        acl: AclMap,
    ) -> Arc<Self> {
        Arc::new(Self {
            engine,
            auth_tokens,
            acl,
            started_at: std::time::Instant::now(),
        })
    }

    fn acl_enabled(&self) -> bool {
        !self.acl.is_empty()
    }

    /// 从请求头解出已认证参与者（令牌表为空 → None = 信任声明模式）。
    ///
    /// 恒时遍历全部令牌再判定，不按 principal 提前返回（CWE-208）。
    fn authenticated_principal(&self, headers: &HeaderMap) -> Result<Option<String>, ApiError> {
        if self.auth_tokens.is_empty() {
            return Ok(None);
        }
        let supplied = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "));
        let Some(supplied) = supplied else {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Unauthorized",
                "缺少 Bearer 令牌".to_owned(),
            ));
        };
        let mut found = None;
        for (principal, token) in &self.auth_tokens {
            if ct_eq(supplied, token) {
                found = Some(principal.clone());
            }
        }
        match found {
            Some(p) => Ok(Some(p)),
            None => Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Unauthorized",
                "令牌不匹配".to_owned(),
            )),
        }
    }

    /// ACL 已启用时必须能确定调用者；令牌表为空属配置非法（daemon 启动已拦截）。
    fn require_principal(&self, headers: &HeaderMap) -> Result<String, ApiError> {
        match self.authenticated_principal(headers)? {
            Some(p) => Ok(p),
            None => Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Unauthorized",
                "ACL 已启用但令牌表为空（配置非法）".to_owned(),
            )),
        }
    }

    fn forbidden(domain: &str, instance: &str) -> ApiError {
        ApiError {
            status: StatusCode::FORBIDDEN,
            detail: ErrorDetail {
                code: "Forbidden".to_owned(),
                detail: serde_json::json!({ "domain": domain, "instance": instance }),
                message: format!("无权访问域实例 {domain}/{instance}"),
            },
        }
    }

    /// 参与者身份校验（§9.4：命令进入引擎前完成）。
    /// 令牌表为空 = 信任声明模式，直接放行。
    fn authorize(&self, participant: &str, headers: &HeaderMap) -> Result<(), ApiError> {
        if self.auth_tokens.is_empty() {
            return Ok(());
        }
        let expected = self.auth_tokens.get(participant).ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Unauthorized",
                format!("参与者 {participant} 无登记令牌"),
            )
        })?;
        let supplied = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "));
        match supplied {
            Some(t) if ct_eq(t, expected) => Ok(()),
            _ => Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Unauthorized",
                format!("参与者 {participant} 令牌缺失或不匹配"),
            )),
        }
    }

    /// 动作端点：令牌校验 +（启用 ACL 时）实例授权。
    fn authorize_action(
        &self,
        participant: &str,
        headers: &HeaderMap,
        key: &InstanceKey,
    ) -> Result<(), ApiError> {
        self.authorize(participant, headers)?;
        if self.acl_enabled() && !acl_allows(&self.acl, participant, &key.domain, &key.instance) {
            return Err(Self::forbidden(&key.domain, &key.instance));
        }
        Ok(())
    }

    /// 实例级端点（生命周期/查询/压缩）：ACL 未启用时保持旧行为；
    /// 启用时从令牌解出参与者并按 `(domain, instance)` 授权。
    fn authorize_key(&self, headers: &HeaderMap, key: &InstanceKey) -> Result<(), ApiError> {
        if !self.acl_enabled() {
            return Ok(());
        }
        let principal = self.require_principal(headers)?;
        if !acl_allows(&self.acl, &principal, &key.domain, &key.instance) {
            return Err(Self::forbidden(&key.domain, &key.instance));
        }
        Ok(())
    }

    /// 定义级端点（如 analyze）：ACL 启用时要求对该域有任一授权。
    fn authorize_domain(&self, headers: &HeaderMap, domain: &str) -> Result<(), ApiError> {
        if !self.acl_enabled() {
            return Ok(());
        }
        let principal = self.require_principal(headers)?;
        if !acl_allows_domain(&self.acl, &principal, domain) {
            return Err(Self::forbidden(domain, "*"));
        }
        Ok(())
    }
}

/// 恒时字符串比较（CWE-208 防护）：比较耗时与字节内容无关，仅依赖较短一侧长度。
/// 长度差异经 `len ^ len` 计入 diff，不等长恒为假。安全审视（2026-09-06）落定。
fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.iter().zip(b) {
        diff |= usize::from(x ^ y);
    }
    diff == 0
}

/// §6.3 结构化错误响应。
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    detail: ErrorDetail,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &str, message: String) -> Self {
        Self {
            status,
            detail: ErrorDetail {
                code: code.to_owned(),
                detail: serde_json::Value::Null,
                message,
            },
        }
    }
}

impl From<EngineError> for ApiError {
    fn from(e: EngineError) -> Self {
        match &e {
            EngineError::InstanceNotFound { domain, instance } => ApiError {
                status: StatusCode::NOT_FOUND,
                detail: ErrorDetail {
                    code: "InstanceNotFound".to_owned(),
                    detail: serde_json::json!({ "domain": domain, "instance": instance }),
                    message: format!("实例未加载：{domain}/{instance}"),
                },
            },
            EngineError::DomainNotDeclared { name } => ApiError {
                status: StatusCode::NOT_FOUND,
                detail: ErrorDetail {
                    code: "DomainNotDeclared".to_owned(),
                    detail: serde_json::json!({ "name": name }),
                    message: format!("域声明未登记：{name}"),
                },
            },
            EngineError::InstanceExists { domain, instance } => ApiError {
                status: StatusCode::CONFLICT,
                detail: ErrorDetail {
                    code: "InstanceExists".to_owned(),
                    detail: serde_json::json!({ "domain": domain, "instance": instance }),
                    message: format!("实例已存在：{domain}/{instance}"),
                },
            },
            EngineError::DomainBusy { domain, instance } => ApiError {
                status: StatusCode::CONFLICT,
                detail: ErrorDetail {
                    code: "DomainBusy".to_owned(),
                    detail: serde_json::json!({ "domain": domain, "instance": instance }),
                    message: format!(
                        "实例 {domain}/{instance} 非空：加载将重置其 Counter，如确认替换请显式 force"
                    ),
                },
            },
            EngineError::Parse(p) => ApiError {
                status: StatusCode::BAD_REQUEST,
                detail: problem_detail(p),
            },
            EngineError::Rule(r) => ApiError {
                status: StatusCode::CONFLICT,
                detail: rule_error_detail(r),
            },
            EngineError::Compose(cs) => ApiError {
                status: StatusCode::CONFLICT,
                detail: conflicts_detail(cs),
            },
            EngineError::Store(StoreError::InvalidDomainName { name }) => ApiError {
                status: StatusCode::BAD_REQUEST,
                detail: ErrorDetail {
                    code: "InvalidDomainName".to_owned(),
                    detail: serde_json::json!({ "name": name }),
                    message: format!(
                        "非法域名：{name:?}（仅允许字母、数字、'-'、'_'，长度 ≤ 128）"
                    ),
                },
            },
            EngineError::Store(StoreError::InvalidInstanceName { name }) => ApiError {
                status: StatusCode::BAD_REQUEST,
                detail: ErrorDetail {
                    code: "InvalidInstanceName".to_owned(),
                    detail: serde_json::json!({ "name": name }),
                    message: format!(
                        "非法实例名：{name:?}（仅允许字母、数字、'-'、'_'，长度 ≤ 128）"
                    ),
                },
            },
            EngineError::Store(s) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Store",
                format!("存储失败：{s}"),
            ),
            EngineError::Recovery(msg) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Recovery",
                format!("启动恢复失败：{msg}"),
            ),
            EngineError::IdempotencyConflict {
                request_id,
                existing,
                requested,
            } => ApiError {
                status: StatusCode::CONFLICT,
                detail: ErrorDetail {
                    code: "IdempotencyConflict".to_owned(),
                    detail: serde_json::json!({
                        "request_id": request_id,
                        "existing_token_id": existing.0,
                        "requested_token_id": requested.0,
                    }),
                    message: format!(
                        "幂等键 {request_id} 已用于消费 token {existing}，与本次请求的 {requested} 冲突"
                    ),
                },
            },
            EngineError::EngineStopped => ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "EngineStopped",
                "引擎已停止".to_owned(),
            ),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(ErrorBody { error: self.detail })).into_response()
    }
}

/// JSON 请求体提取器：解析失败 → §6.3 结构化 400。
pub struct JsonReq<T>(pub T);

impl<T, S> FromRequest<S> for JsonReq<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(JsonReq(v)),
            Err(r) => Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "BadRequest",
                json_rejection_text(r),
            )),
        }
    }
}

fn json_rejection_text(r: JsonRejection) -> String {
    match r {
        JsonRejection::JsonSyntaxError(e) => format!("JSON 语法错误：{e}"),
        JsonRejection::JsonDataError(e) => format!("JSON 类型不符：{e}"),
        JsonRejection::MissingJsonContentType(_) => {
            "缺少 Content-Type: application/json".to_owned()
        }
        JsonRejection::BytesRejection(e) => format!("请求体读取失败：{e}"),
        _ => "请求体无效".to_owned(),
    }
}

// ============ 查询参数 ============

#[derive(Deserialize)]
struct TypeQuery {
    #[serde(rename = "type", default)]
    ty: Option<String>,
}

#[derive(Deserialize)]
struct ParticipantQuery {
    participant: String,
}

// ============ 路由 ============

pub fn router(state: Arc<ServerState>) -> Router {
    Router::new()
        // 运维端点（D25 子项 2）：探活/就绪/指标，不校验令牌（仅本地/内网暴露）
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/metrics", get(metrics))
        .route("/domains", post(load_domain).get(list_domains))
        .route("/validate", post(validate_declaration))
        .route("/domains/{d}", any(domain_entry))
        .route("/domains/{d}/instances", post(instantiate))
        .route("/domains/{d}/instances/{i}", any(instance_entry))
        // default 实例端点（等价 `/domains/{d}/instances/default/...`）
        .route("/domains/{d}/seeds", post(seed))
        .route("/domains/{d}/tokens", post(produce))
        .route("/domains/{d}/counter", get(counter))
        .route("/domains/{d}/archive", get(archive))
        .route("/domains/{d}/tokens/{id}/read", post(read_token))
        .route("/domains/{d}/tokens/{id}/consume", post(consume_token))
        .route("/domains/{d}/gates/{participant}", get(gates))
        .route("/domains/{d}/enabled", get(enabled))
        .route("/domains/{d}/status", get(status))
        .route("/domains/{d}/events", get(events))
        // 读审计只读视图（D25 子项 3；default 实例与实例级镜像）
        .route("/domains/{d}/read-audit", get(read_audit_default))
        .route(
            "/domains/{d}/instances/{i}/read-audit",
            get(read_audit_instance),
        )
        // 日志前缀压缩（D25 子项 1；default 实例与实例级镜像）
        .route("/domains/{d}/compact", post(compact_default))
        // 实例级镜像组（D27）
        .route("/domains/{d}/instances/{i}/seeds", post(seed_instance))
        .route("/domains/{d}/instances/{i}/tokens", post(produce_instance))
        .route("/domains/{d}/instances/{i}/counter", get(counter_instance))
        .route("/domains/{d}/instances/{i}/archive", get(archive_instance))
        .route(
            "/domains/{d}/instances/{i}/tokens/{id}/read",
            post(read_token_instance),
        )
        .route(
            "/domains/{d}/instances/{i}/tokens/{id}/consume",
            post(consume_token_instance),
        )
        .route(
            "/domains/{d}/instances/{i}/gates/{participant}",
            get(gates_instance),
        )
        .route("/domains/{d}/instances/{i}/enabled", get(enabled_instance))
        .route("/domains/{d}/instances/{i}/status", get(status_instance))
        .route("/domains/{d}/instances/{i}/events", get(events_instance))
        .route("/domains/{d}/instances/{i}/compact", post(compact_instance))
        .fallback(not_found)
        .with_state(state)
}

// ============ 运维端点（D25 子项 2：health / ready / metrics，不校验令牌） ============

/// 存活探针（liveness）：进程在服务即 200，不依赖引擎状态。
async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

/// 就绪探针（readiness）：引擎可应答（启动恢复完成且未停机）→ 200，否则 503。
/// 探针 = 一次 `list_domains` 往返；恢复失败时引擎以 `Recovery` 统一应答。
async fn ready(State(st): State<Arc<ServerState>>) -> Response {
    match st.engine.list_domains().await {
        Ok(listings) => {
            let instances: usize = listings.iter().map(|l| l.instances.len()).sum();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ready",
                    "domains": listings.len(),
                    "instances": instances,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "status": "not_ready", "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// 运行指标（Prometheus 文本暴露格式）。
/// 标签值取自域名/实例名（谓词 `[A-Za-z0-9_-]`，无需转义）。
/// 仅上报已加载（内存）实例：磁盘存在但未加载者由 `status` 报 `InstanceNotFound`，跳过。
async fn metrics(State(st): State<Arc<ServerState>>) -> Response {
    let listings = match st.engine.list_domains().await {
        Ok(l) => l,
        Err(e) => {
            return text_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &format!("# 引擎不可用：{e}\n"),
            )
        }
    };

    let mut loaded: Vec<(InstanceKey, StatusView)> = Vec::new();
    for l in &listings {
        for name in &l.instances {
            let key = InstanceKey::new(l.name.clone(), name.clone());
            match st.engine.status(key.clone()).await {
                Ok(v) => loaded.push((key, v)),
                Err(EngineError::InstanceNotFound { .. }) => {}
                Err(e) => {
                    return text_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        &format!("# 引擎查询失败：{e}\n"),
                    );
                }
            }
        }
    }

    let mut out = String::new();
    out.push_str("# HELP pact_domains_declared 已登记的域声明数\n");
    out.push_str("# TYPE pact_domains_declared gauge\n");
    out.push_str(&format!("pact_domains_declared {}\n", listings.len()));
    out.push_str("# HELP pact_instances_loaded 已加载的域实例数\n");
    out.push_str("# TYPE pact_instances_loaded gauge\n");
    out.push_str(&format!("pact_instances_loaded {}\n", loaded.len()));
    out.push_str("# HELP pact_counter_size 域实例 Counter 当前 token 数\n");
    out.push_str("# TYPE pact_counter_size gauge\n");
    for (k, v) in &loaded {
        out.push_str(&format!(
            "pact_counter_size{{domain=\"{}\",instance=\"{}\"}} {}\n",
            k.domain, k.instance, v.counter_size
        ));
    }
    out.push_str("# HELP pact_archive_size 域实例 Archive 条目数\n");
    out.push_str("# TYPE pact_archive_size gauge\n");
    for (k, v) in &loaded {
        out.push_str(&format!(
            "pact_archive_size{{domain=\"{}\",instance=\"{}\"}} {}\n",
            k.domain, k.instance, v.archive_size
        ));
    }
    out.push_str("# HELP pact_terminated 域实例是否满足 Rule-T 终止条件（1/0）\n");
    out.push_str("# TYPE pact_terminated gauge\n");
    for (k, v) in &loaded {
        out.push_str(&format!(
            "pact_terminated{{domain=\"{}\",instance=\"{}\"}} {}\n",
            k.domain,
            k.instance,
            u8::from(v.terminated)
        ));
    }
    out.push_str("# HELP pact_uptime_seconds 进程运行时长\n");
    out.push_str("# TYPE pact_uptime_seconds gauge\n");
    out.push_str(&format!(
        "pact_uptime_seconds {:.3}\n",
        st.started_at.elapsed().as_secs_f64()
    ));

    text_response(StatusCode::OK, &out)
}

/// 文本响应（Prometheus 暴露格式用，`text/plain; version=0.0.4`）。
fn text_response(status: StatusCode, body: &str) -> Response {
    let mut resp = Response::new(Body::from(body.to_owned()));
    *resp.status_mut() = status;
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    resp
}

/// 默认实例键（`/domains/{d}` 路径族语义，FR19 兼容）。
fn default_key(domain: &str) -> InstanceKey {
    InstanceKey::new(domain, DEFAULT_INSTANCE)
}

/// `/domains/{d}` 统一入口：matchit 不支持同段“参数+字面量”，
/// 故 `:analyze` / `:compose`（§9.1）在此按段内冒号分发。
/// 其余语义作用于**默认实例**。
async fn domain_entry(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    req: Request,
) -> axum::response::Response {
    let method = req.method().clone();
    let headers = req.headers().clone();
    if let Some((name, suffix)) = d.split_once(':') {
        if name.is_empty() {
            return not_found_path(&d);
        }
        match (&method, suffix) {
            (&Method::POST, "analyze") => {
                match analyze_domain(State(st), Path(name.to_owned()), headers).await {
                    Ok(body) => body.into_response(),
                    Err(e) => e.into_response(),
                }
            }
            (&Method::POST, "compose") => {
                match JsonReq::<ComposeReq>::from_request(req, &()).await {
                    Ok(j) => {
                        match compose_domain(State(st), Path(name.to_owned()), headers, j).await {
                            Ok(resp) => resp.into_response(),
                            Err(e) => e.into_response(),
                        }
                    }
                    Err(e) => e.into_response(),
                }
            }
            (_, "analyze" | "compose") => ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "MethodNotAllowed",
                format!("方法 {method} 不适用于 :{suffix}（仅 POST）"),
            )
            .into_response(),
            _ => not_found_path(&d),
        }
    } else {
        match method {
            Method::GET => match domain_info(State(st), Path(d), headers).await {
                Ok(body) => body.into_response(),
                Err(e) => e.into_response(),
            },
            Method::DELETE => match unload_domain(State(st), Path(d), headers).await {
                Ok(code) => code.into_response(),
                Err(e) => e.into_response(),
            },
            _ => ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "MethodNotAllowed",
                format!("方法 {method} 不适用于 /domains/{{d}}（仅 GET/DELETE）"),
            )
            .into_response(),
        }
    }
}

/// `/domains/{d}/instances/{i}`：GET 状态 / DELETE 卸载。
async fn instance_entry(
    State(st): State<Arc<ServerState>>,
    Path((d, i)): Path<(String, String)>,
    req: Request,
) -> axum::response::Response {
    let method = req.method().clone();
    let headers = req.headers().clone();
    match method {
        Method::GET => match instance_info(State(st), Path((d, i)), headers).await {
            Ok(body) => body.into_response(),
            Err(e) => e.into_response(),
        },
        Method::DELETE => match unload_instance(State(st), Path((d, i)), headers).await {
            Ok(code) => code.into_response(),
            Err(e) => e.into_response(),
        },
        _ => ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "MethodNotAllowed",
            format!("方法 {method} 不适用于实例端点（仅 GET/DELETE）"),
        )
        .into_response(),
    }
}

async fn not_found(req: Request) -> axum::response::Response {
    not_found_path(req.uri().path())
}

fn not_found_path(path: &str) -> axum::response::Response {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "NotFound",
        format!("路径不存在：{path}"),
    )
    .into_response()
}

// ============ 处理器 ============

async fn load_domain(
    State(st): State<Arc<ServerState>>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<LoadDomainReq>,
) -> Result<(StatusCode, Json<pact_engine::views::DomainSummary>), ApiError> {
    let instance = req.instance.unwrap_or_else(|| DEFAULT_INSTANCE.to_owned());
    st.authorize_key(&headers, &InstanceKey::new(&req.name, &instance))?;
    let summary = st
        .engine
        .load_domain(
            &req.name,
            &req.declaration,
            &instance,
            req.force.unwrap_or(false),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(summary)))
}

/// `POST /domains/{d}/instances`：在已登记声明上创建额外实例。
async fn instantiate(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<InstantiateReq>,
) -> Result<(StatusCode, Json<pact_engine::views::DomainSummary>), ApiError> {
    st.authorize_key(&headers, &InstanceKey::new(&d, &req.instance))?;
    let summary = st.engine.instantiate(&d, &req.instance).await?;
    Ok((StatusCode::CREATED, Json(summary)))
}

/// `POST /domains/{d}/instances/{i}/compact`：日志前缀压缩（D25 子项 1）。
/// 写全量自足快照后截断日志中已被覆盖的前缀；不改变实例状态与 seq 续接点。
async fn compact_instance(
    State(st): State<Arc<ServerState>>,
    Path((d, i)): Path<(String, String)>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<CompactReq>,
) -> Result<Json<pact_engine::views::CompactionOutcome>, ApiError> {
    let key = InstanceKey::new(d, i);
    st.authorize_key(&headers, &key)?;
    let out = st.engine.compact(key, req.keep_last).await?;
    Ok(Json(out))
}

/// `POST /domains/{d}/compact`：对默认实例压缩（D27 兼容路径）。
async fn compact_default(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<CompactReq>,
) -> Result<Json<pact_engine::views::CompactionOutcome>, ApiError> {
    let key = default_key(&d);
    st.authorize_key(&headers, &key)?;
    let out = st.engine.compact(key, req.keep_last).await?;
    Ok(Json(out))
}

/// `GET /domains`：启用 ACL 时仅返回调用者可访问的实例（空域被剔除）。
async fn list_domains(
    State(st): State<Arc<ServerState>>,
    headers: HeaderMap,
) -> Result<Json<DomainsResp>, ApiError> {
    let mut domains = st.engine.list_domains().await?;
    if st.acl_enabled() {
        let principal = st.require_principal(&headers)?;
        for listing in &mut domains {
            listing
                .instances
                .retain(|i| acl_allows(&st.acl, &principal, &listing.name, i));
        }
        domains.retain(|l| !l.instances.is_empty());
    }
    Ok(Json(DomainsResp { domains }))
}

/// 校验声明文本良构（§1.4.4），不加载。
async fn validate_declaration(
    State(st): State<Arc<ServerState>>,
    JsonReq(req): JsonReq<ValidateReq>,
) -> Result<Json<ValidateResp>, ApiError> {
    let domain = st.engine.validate(&req.declaration).await?;
    let mut participants = BTreeSet::new();
    for (p, _, _) in domain.gates.keys() {
        participants.insert(p.0.clone());
    }
    for (p, _) in &domain.seed_authority {
        participants.insert(p.0.clone());
    }
    Ok(Json(ValidateResp {
        ok: true,
        types: domain.types.iter().map(|t| t.0.clone()).collect(),
        participants: participants.into_iter().collect(),
        gates: domain.gates.len(),
        seed_authority: domain.seed_authority.len(),
    }))
}

async fn domain_info(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
) -> Result<Json<DomainInfo>, ApiError> {
    let key = default_key(&d);
    st.authorize_key(&headers, &key)?;
    domain_info_inner(&st, key).await
}

async fn instance_info(
    State(st): State<Arc<ServerState>>,
    Path((d, i)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<DomainInfo>, ApiError> {
    let key = InstanceKey::new(d, i);
    st.authorize_key(&headers, &key)?;
    domain_info_inner(&st, key).await
}

async fn domain_info_inner(
    st: &Arc<ServerState>,
    key: InstanceKey,
) -> Result<Json<DomainInfo>, ApiError> {
    let s = st.engine.status(key.clone()).await?;
    Ok(Json(DomainInfo {
        name: key.domain,
        instance: key.instance,
        counter_size: s.counter_size,
        archive_size: s.archive_size,
        terminated: s.terminated,
    }))
}

async fn unload_domain(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let key = default_key(&d);
    st.authorize_key(&headers, &key)?;
    unload_inner(&st, key).await
}

async fn unload_instance(
    State(st): State<Arc<ServerState>>,
    Path((d, i)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let key = InstanceKey::new(d, i);
    st.authorize_key(&headers, &key)?;
    unload_inner(&st, key).await
}

async fn unload_inner(st: &Arc<ServerState>, key: InstanceKey) -> Result<StatusCode, ApiError> {
    st.engine.unload_domain(key).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn analyze_domain(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
) -> Result<Json<pact_engine::views::AnalysisReport>, ApiError> {
    st.authorize_domain(&headers, &d)?;
    Ok(Json(st.engine.analyze_loaded(&d).await?))
}

async fn compose_domain(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<ComposeReq>,
) -> Result<(StatusCode, Json<pact_engine::views::DomainSummary>), ApiError> {
    st.authorize_key(&headers, &default_key(&d))?;
    let summary = st
        .engine
        .compose_and_load(&d, req.fragments, DEFAULT_INSTANCE)
        .await?;
    Ok((StatusCode::CREATED, Json(summary)))
}

async fn seed(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<SeedReq>,
) -> Result<(StatusCode, Json<TokenResp>), ApiError> {
    seed_inner(&st, default_key(&d), headers, req).await
}

async fn seed_instance(
    State(st): State<Arc<ServerState>>,
    Path((d, i)): Path<(String, String)>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<SeedReq>,
) -> Result<(StatusCode, Json<TokenResp>), ApiError> {
    seed_inner(&st, InstanceKey::new(d, i), headers, req).await
}

async fn seed_inner(
    st: &Arc<ServerState>,
    key: InstanceKey,
    headers: HeaderMap,
    req: SeedReq,
) -> Result<(StatusCode, Json<TokenResp>), ApiError> {
    st.authorize_action(&req.participant, &headers, &key)?;
    let token = st
        .engine
        .seed(
            key,
            ParticipantName(req.participant),
            TypeName(req.ty),
            thing_ref(req.thing_ref),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(TokenResp { token })))
}

async fn produce(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<ProduceReq>,
) -> Result<(StatusCode, Json<TokenResp>), ApiError> {
    produce_inner(&st, default_key(&d), headers, req).await
}

async fn produce_instance(
    State(st): State<Arc<ServerState>>,
    Path((d, i)): Path<(String, String)>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<ProduceReq>,
) -> Result<(StatusCode, Json<TokenResp>), ApiError> {
    produce_inner(&st, InstanceKey::new(d, i), headers, req).await
}

async fn produce_inner(
    st: &Arc<ServerState>,
    key: InstanceKey,
    headers: HeaderMap,
    req: ProduceReq,
) -> Result<(StatusCode, Json<TokenResp>), ApiError> {
    st.authorize_action(&req.participant, &headers, &key)?;
    let predecessors = req.predecessors.into_iter().map(TokenId).collect();
    let token = st
        .engine
        .produce(
            key,
            ParticipantName(req.participant),
            TypeName(req.ty),
            predecessors,
            thing_ref(req.thing_ref),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(TokenResp { token })))
}

async fn counter(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
    Query(q): Query<TypeQuery>,
) -> Result<Json<pact_engine::views::CounterView>, ApiError> {
    counter_inner(&st, default_key(&d), headers, q).await
}

async fn counter_instance(
    State(st): State<Arc<ServerState>>,
    Path((d, i)): Path<(String, String)>,
    headers: HeaderMap,
    Query(q): Query<TypeQuery>,
) -> Result<Json<pact_engine::views::CounterView>, ApiError> {
    counter_inner(&st, InstanceKey::new(d, i), headers, q).await
}

async fn counter_inner(
    st: &Arc<ServerState>,
    key: InstanceKey,
    headers: HeaderMap,
    q: TypeQuery,
) -> Result<Json<pact_engine::views::CounterView>, ApiError> {
    st.authorize_key(&headers, &key)?;
    Ok(Json(st.engine.counter(key, q.ty.map(TypeName)).await?))
}

async fn archive(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
    Query(q): Query<TypeQuery>,
) -> Result<Json<pact_engine::views::ArchiveView>, ApiError> {
    archive_inner(&st, default_key(&d), headers, q).await
}

async fn archive_instance(
    State(st): State<Arc<ServerState>>,
    Path((d, i)): Path<(String, String)>,
    headers: HeaderMap,
    Query(q): Query<TypeQuery>,
) -> Result<Json<pact_engine::views::ArchiveView>, ApiError> {
    archive_inner(&st, InstanceKey::new(d, i), headers, q).await
}

async fn archive_inner(
    st: &Arc<ServerState>,
    key: InstanceKey,
    headers: HeaderMap,
    q: TypeQuery,
) -> Result<Json<pact_engine::views::ArchiveView>, ApiError> {
    st.authorize_key(&headers, &key)?;
    Ok(Json(st.engine.archive(key, q.ty.map(TypeName)).await?))
}

async fn read_token(
    State(st): State<Arc<ServerState>>,
    Path((d, id)): Path<(String, String)>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<ParticipantReq>,
) -> Result<Json<TokenResp>, ApiError> {
    read_inner(&st, default_key(&d), id, headers, req).await
}

async fn read_token_instance(
    State(st): State<Arc<ServerState>>,
    Path((d, i, id)): Path<(String, String, String)>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<ParticipantReq>,
) -> Result<Json<TokenResp>, ApiError> {
    read_inner(&st, InstanceKey::new(d, i), id, headers, req).await
}

async fn read_inner(
    st: &Arc<ServerState>,
    key: InstanceKey,
    id: String,
    headers: HeaderMap,
    req: ParticipantReq,
) -> Result<Json<TokenResp>, ApiError> {
    st.authorize_action(&req.participant, &headers, &key)?;
    let token = st
        .engine
        .read(key, ParticipantName(req.participant), TokenId(id))
        .await?;
    Ok(Json(TokenResp { token }))
}

async fn consume_token(
    State(st): State<Arc<ServerState>>,
    Path((d, id)): Path<(String, String)>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<ConsumeReq>,
) -> Result<Json<EntryResp>, ApiError> {
    consume_inner(&st, default_key(&d), id, headers, req).await
}

async fn consume_token_instance(
    State(st): State<Arc<ServerState>>,
    Path((d, i, id)): Path<(String, String, String)>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<ConsumeReq>,
) -> Result<Json<EntryResp>, ApiError> {
    consume_inner(&st, InstanceKey::new(d, i), id, headers, req).await
}

async fn consume_inner(
    st: &Arc<ServerState>,
    key: InstanceKey,
    id: String,
    headers: HeaderMap,
    req: ConsumeReq,
) -> Result<Json<EntryResp>, ApiError> {
    st.authorize_action(&req.participant, &headers, &key)?;
    let entry = st
        .engine
        .consume(
            key,
            ParticipantName(req.participant),
            TokenId(id),
            req.request_id,
        )
        .await?;
    Ok(Json(EntryResp { entry }))
}

async fn gates(
    State(st): State<Arc<ServerState>>,
    Path((d, participant)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<GatesResp>, ApiError> {
    gates_inner(&st, default_key(&d), participant, headers).await
}

async fn gates_instance(
    State(st): State<Arc<ServerState>>,
    Path((d, i, participant)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Json<GatesResp>, ApiError> {
    gates_inner(&st, InstanceKey::new(d, i), participant, headers).await
}

async fn gates_inner(
    st: &Arc<ServerState>,
    key: InstanceKey,
    participant: String,
    headers: HeaderMap,
) -> Result<Json<GatesResp>, ApiError> {
    st.authorize_action(&participant, &headers, &key)?;
    let gates = st
        .engine
        .my_gates(key, ParticipantName(participant))
        .await?;
    Ok(Json(GatesResp { gates }))
}

async fn enabled(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
    Query(q): Query<ParticipantQuery>,
) -> Result<Json<EnabledResp>, ApiError> {
    enabled_inner(&st, default_key(&d), headers, q).await
}

async fn enabled_instance(
    State(st): State<Arc<ServerState>>,
    Path((d, i)): Path<(String, String)>,
    headers: HeaderMap,
    Query(q): Query<ParticipantQuery>,
) -> Result<Json<EnabledResp>, ApiError> {
    enabled_inner(&st, InstanceKey::new(d, i), headers, q).await
}

async fn enabled_inner(
    st: &Arc<ServerState>,
    key: InstanceKey,
    headers: HeaderMap,
    q: ParticipantQuery,
) -> Result<Json<EnabledResp>, ApiError> {
    st.authorize_action(&q.participant, &headers, &key)?;
    let options = st
        .engine
        .enabled(key, ParticipantName(q.participant))
        .await?
        .into_iter()
        .map(EnabledOption::from)
        .collect();
    Ok(Json(EnabledResp { options }))
}

// ============ 读审计视图（D25 子项 3） ============

/// 读审计查询参数：`limit` 缺省 100，夹取到 `[1, 1000]`。
#[derive(Debug, Deserialize)]
struct ReadAuditQuery {
    #[serde(default)]
    limit: Option<usize>,
}

const READ_AUDIT_DEFAULT_LIMIT: usize = 100;
const READ_AUDIT_MAX_LIMIT: usize = 1000;

fn audit_limit(q: &ReadAuditQuery) -> usize {
    q.limit
        .unwrap_or(READ_AUDIT_DEFAULT_LIMIT)
        .clamp(1, READ_AUDIT_MAX_LIMIT)
}

async fn read_audit_default(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
    Query(q): Query<ReadAuditQuery>,
) -> Result<Json<ReadAuditResp>, ApiError> {
    read_audit_inner(&st, default_key(&d), headers, q).await
}

async fn read_audit_instance(
    State(st): State<Arc<ServerState>>,
    Path((d, i)): Path<(String, String)>,
    headers: HeaderMap,
    Query(q): Query<ReadAuditQuery>,
) -> Result<Json<ReadAuditResp>, ApiError> {
    read_audit_inner(&st, InstanceKey::new(d, i), headers, q).await
}

async fn read_audit_inner(
    st: &Arc<ServerState>,
    key: InstanceKey,
    headers: HeaderMap,
    q: ReadAuditQuery,
) -> Result<Json<ReadAuditResp>, ApiError> {
    st.authorize_key(&headers, &key)?;
    let records = st.engine.read_audit(key, audit_limit(&q)).await?;
    Ok(Json(ReadAuditResp { records }))
}

async fn status(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
) -> Result<Json<StatusView>, ApiError> {
    status_inner(&st, default_key(&d), headers).await
}

async fn status_instance(
    State(st): State<Arc<ServerState>>,
    Path((d, i)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<StatusView>, ApiError> {
    status_inner(&st, InstanceKey::new(d, i), headers).await
}

async fn status_inner(
    st: &Arc<ServerState>,
    key: InstanceKey,
    headers: HeaderMap,
) -> Result<Json<StatusView>, ApiError> {
    st.authorize_key(&headers, &key)?;
    Ok(Json(st.engine.status(key).await?))
}

/// §6.4 SSE：Counter 变更通知流。
async fn events(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    events_inner(&st, default_key(&d), headers).await
}

async fn events_instance(
    State(st): State<Arc<ServerState>>,
    Path((d, i)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    events_inner(&st, InstanceKey::new(d, i), headers).await
}

async fn events_inner(
    st: &Arc<ServerState>,
    key: InstanceKey,
    headers: HeaderMap,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    st.authorize_key(&headers, &key)?;
    let rx = st.engine.subscribe(key).await?;
    let stream = BroadcastStream::new(rx).filter_map(|r| async move {
        r.ok().map(|n| {
            Ok(Event::default()
                .json_data(n)
                .unwrap_or_else(|_| Event::default()))
        })
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn thing_ref(s: Option<String>) -> ThingRef {
    ThingRef(s)
}

// ============ 监听 ============

/// 在途连接计数器（优雅停机用）：停机时停止接受新连接，等待在途请求完成（有界）。
#[derive(Clone, Default)]
pub struct ConnTracker {
    active: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}

impl ConnTracker {
    /// 进入连接作用域；返回的 guard 析构时计数归还并唤醒等待者。
    pub fn guard(&self) -> ConnGuard {
        self.active.fetch_add(1, Ordering::SeqCst);
        ConnGuard {
            tracker: self.clone(),
        }
    }

    /// 当前在途连接数。
    pub fn active(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }

    /// 等待在途连接归零。
    pub async fn wait_idle(&self) {
        loop {
            if self.active() == 0 {
                return;
            }
            self.notify.notified().await;
        }
    }
}

/// 连接存活作用域 guard（drop 即归还计数）。
pub struct ConnGuard {
    tracker: ConnTracker,
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        // notify_one 会留存一个许可，避免 wait_idle 在「检查 → 挂起」之间错过唤醒
        self.tracker.active.fetch_sub(1, Ordering::SeqCst);
        self.tracker.notify.notify_one();
    }
}

/// unix socket 监听循环（默认；单机安全）。启动前移除旧套接字文件。
/// `shutdown` 置位后停止接受新连接并返回（在途连接由 `tracker` 跟踪）。
pub async fn serve_unix(
    app: Router,
    path: &FsPath,
    mut shutdown: watch::Receiver<bool>,
    tracker: ConnTracker,
) -> std::io::Result<()> {
    let _ = std::fs::remove_file(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    loop {
        tokio::select! {
            r = listener.accept() => {
                let (stream, _) = r?;
                spawn_connection(app.clone(), hyper_util::rt::TokioIo::new(stream), tracker.guard());
            }
            _ = wait_shutdown(&mut shutdown) => break,
        }
    }
    Ok(())
}

/// TCP 监听循环（可选，建议 127.0.0.1）。
pub async fn serve_tcp(
    app: Router,
    addr: &str,
    mut shutdown: watch::Receiver<bool>,
    tracker: ConnTracker,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    loop {
        tokio::select! {
            r = listener.accept() => {
                let (stream, _) = r?;
                spawn_connection(app.clone(), hyper_util::rt::TokioIo::new(stream), tracker.guard());
            }
            _ = wait_shutdown(&mut shutdown) => break,
        }
    }
    Ok(())
}

/// 等待停机信号（已置位立即返回；发送端 drop 亦视为停机）。
async fn wait_shutdown(rx: &mut watch::Receiver<bool>) {
    if *rx.borrow() {
        return;
    }
    let _ = rx.changed().await;
}

/// 单连接服务（hyper-util auto：HTTP/1.1）。`guard` 随连接结束释放。
fn spawn_connection<I>(app: Router, io: I, guard: ConnGuard)
where
    I: hyper::rt::Read + hyper::rt::Write + Send + Unpin + 'static,
{
    tokio::spawn(async move {
        let _guard = guard;
        let svc = hyper_util::service::TowerToHyperService::new(app.into_service());
        let builder =
            hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
        // 连接级错误（客户端断开等）不影响 daemon 存活
        let _ = builder.serve_connection(io, svc).await;
    });
}

#[cfg(test)]
mod tests {
    use super::ct_eq;

    #[test]
    fn ct_eq_matches_only_identical_strings() {
        assert!(ct_eq("tok-abc123", "tok-abc123"));
        assert!(!ct_eq("tok-abc123", "tok-abc124"));
        assert!(!ct_eq("tok-abc123", "tok-abc1234")); // 不等长恒为假
        assert!(!ct_eq("tok-abc123", "")); // 空串不等
        assert!(!ct_eq("", "tok-abc123"));
        assert!(ct_eq("", "")); // 空表信任模式下不会走到此处，仅锁定语义
    }
}
