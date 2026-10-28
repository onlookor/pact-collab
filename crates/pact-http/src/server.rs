//! REST daemon（设计 §9.2）：axum 路由 + unix socket / TCP 双监听。
//!
//! 被拒动作返回 4xx + §6.3 结构化错误体；错误码 = RuleError / Problem 变体名。

use crate::config::{acl_allows, acl_allows_domain, acl_allows_wildcard, AclMap};
use crate::dto::{
    conflicts_detail, problem_detail, rule_error_detail, ActivateReq, ActiveResp, CompactReq,
    ComposeReq, ConsumeReq, DomainInfo, DomainsResp, EnabledOption, EnabledResp, EntryResp,
    ErrorBody, ErrorDetail, FreezeReq, GatesResp, HandoffReq, InstantiateReq, LoadDomainReq,
    ParticipantReq, ProduceReq, ReadAuditResp, SeedReq, TokenResp, ValidateReq, ValidateResp,
};
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post, put};
use axum::{Json, Router};
use futures::StreamExt;
use pact_core::{ParticipantName, ThingRef, TokenId, TypeName};
use pact_engine::views::StatusView;
use pact_engine::{Engine, EngineError, InstanceKey, DEFAULT_INSTANCE};
use pact_store::{StoreError, ViewName};
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
    /// D34：参与者 → Ed25519 公钥；非空即启用请求签名。
    auth_keys: BTreeMap<String, [u8; 32]>,
    /// 域级授权表（FR17/FR19）；空表 = 不启用 ACL（向后兼容）。
    acl: AclMap,
    /// D36：MCP OIDC 验证器（仅 `/mcp` 使用）。
    oidc: Option<Arc<crate::oidc::OidcValidator>>,
    /// D36：对外基址（RFC 9728 元数据）。
    public_url: Option<String>,
    /// D42：Counter/Archive 的票面可见性 profile（RefBindings）。
    visibility: crate::visibility::VisibilityProfiles,
    /// D35：进程内能力令牌（MCP → 本地 REST）。
    internal_token: [u8; 32],
    /// D34：nonce 防重放缓存。
    nonces: tokio::sync::Mutex<crate::sign::NonceCache>,
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
        Self::with_identity(engine, auth_tokens, BTreeMap::new(), acl, None, None)
    }

    pub fn with_identity(
        engine: Engine,
        auth_tokens: BTreeMap<String, String>,
        auth_keys: BTreeMap<String, [u8; 32]>,
        acl: AclMap,
        oidc: Option<Arc<crate::oidc::OidcValidator>>,
        public_url: Option<String>,
    ) -> Arc<Self> {
        Self::with_identity_and_visibility(
            engine,
            auth_tokens,
            auth_keys,
            acl,
            oidc,
            public_url,
            crate::visibility::VisibilityProfiles::empty(),
        )
    }

    /// D42：带票面可见性 profile 的构造；其余参数同 `with_identity`。
    pub fn with_identity_and_visibility(
        engine: Engine,
        auth_tokens: BTreeMap<String, String>,
        auth_keys: BTreeMap<String, [u8; 32]>,
        acl: AclMap,
        oidc: Option<Arc<crate::oidc::OidcValidator>>,
        public_url: Option<String>,
        visibility: crate::visibility::VisibilityProfiles,
    ) -> Arc<Self> {
        Arc::new(Self {
            engine,
            auth_tokens,
            auth_keys,
            acl,
            oidc,
            public_url,
            visibility,
            internal_token: random_internal_token(),
            nonces: tokio::sync::Mutex::new(crate::sign::NonceCache::default()),
            started_at: std::time::Instant::now(),
        })
    }

    fn acl_enabled(&self) -> bool {
        !self.acl.is_empty()
    }

    pub fn signing_enabled(&self) -> bool {
        !self.auth_keys.is_empty()
    }

    pub fn auth_key(&self, participant: &str) -> Option<&[u8; 32]> {
        self.auth_keys.get(participant)
    }

    pub fn internal_token(&self) -> [u8; 32] {
        self.internal_token
    }

    pub fn internal_token_hex(&self) -> String {
        let mut out = String::with_capacity(64);
        for b in self.internal_token {
            use std::fmt::Write as _;
            let _ = write!(out, "{b:02x}");
        }
        out
    }

    /// 记录 nonce；重复或缓存满返回 false（429）。
    pub async fn accept_nonce(&self, nonce: &str) -> bool {
        self.nonces
            .lock()
            .await
            .check_and_insert(nonce, std::time::Instant::now())
    }

    pub fn has_static_tokens(&self) -> bool {
        !self.auth_tokens.is_empty()
    }

    pub fn static_token_matches(&self, supplied: &str) -> Option<String> {
        let mut found = None;
        for (principal, token) in &self.auth_tokens {
            if ct_eq(supplied, token) {
                found = Some(principal.clone());
            }
        }
        found
    }

    pub fn oidc(&self) -> Option<&Arc<crate::oidc::OidcValidator>> {
        self.oidc.as_ref()
    }

    pub fn public_url(&self) -> Option<&str> {
        self.public_url.as_deref()
    }

    /// 从请求头解出已认证参与者（令牌表为空 → None = 信任声明模式）。
    ///
    /// 恒时遍历全部令牌再判定，不按 principal 提前返回（CWE-208）。
    fn authenticated_principal(&self, headers: &HeaderMap) -> Result<Option<String>, ApiError> {
        // D34：签名中间件写入的可信头优先。
        if let Some(p) = headers
            .get(crate::sign::HEADER_VERIFIED_PRINCIPAL)
            .and_then(|v| v.to_str().ok())
        {
            return Ok(Some(p.to_owned()));
        }
        if self.auth_tokens.is_empty() {
            if self.signing_enabled() {
                return Err(ApiError::new(
                    StatusCode::UNAUTHORIZED,
                    "Unauthorized",
                    "缺少请求签名".to_owned(),
                ));
            }
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
                "缺少 Bearer 令牌或请求签名".to_owned(),
            ));
        };
        match self.static_token_matches(supplied) {
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

    /// D42：视图调用者身份。令牌模式要求 Bearer（匿名 401）；信任模式为 None。
    fn view_principal(&self, headers: &HeaderMap) -> Result<Option<String>, ApiError> {
        self.authenticated_principal(headers)
    }

    /// D42：调用者在该实例上可无条件读取的类型集合（reads/consumes）。
    async fn readable_types(
        &self,
        key: &InstanceKey,
        principal: Option<&str>,
    ) -> Result<BTreeSet<TypeName>, ApiError> {
        let Some(principal) = principal else {
            return Ok(BTreeSet::new());
        };
        let clauses = self
            .engine
            .my_gates(key.clone(), ParticipantName(principal.to_owned()))
            .await?;
        Ok(clauses
            .into_iter()
            .filter(|clause| {
                clause.condition.is_none()
                    && matches!(
                        clause.action,
                        pact_core::Action::Read | pact_core::Action::Consume
                    )
            })
            .map(|clause| clause.ty)
            .collect())
    }

    /// D43：视图级读审计。开启 `read_audit` 时由引擎 actor 落盘；写失败上抛，
    /// 调用方据此拒绝本次视图响应（fail-closed）。
    async fn record_view_audit(
        &self,
        key: &InstanceKey,
        participant: Option<String>,
        view: ViewName,
        type_filter: Option<String>,
        types: Vec<pact_store::ViewAuditType>,
    ) -> Result<(), ApiError> {
        self.engine
            .record_view_audit(key.clone(), participant, view, type_filter, types)
            .await?;
        Ok(())
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
        if self.auth_tokens.is_empty() && !self.signing_enabled() {
            return Ok(());
        }
        let identity = self.require_principal(headers)?;
        if identity != participant {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Unauthorized",
                format!("请求身份与参与者 {participant} 不一致"),
            ));
        }
        Ok(())
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

    /// 短路由 `/domains/{d}/...` 的实例解析：活动实例指针（无指针 → `default`）。
    async fn resolve_key(&self, domain: &str) -> Result<InstanceKey, ApiError> {
        let instance = self.engine.active_instance(domain).await?;
        Ok(InstanceKey::new(domain, &instance))
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

    /// 控制面端点认证（#3）：只要配置了任何身份（Bearer 令牌或请求签名），
    /// 就必须能认证出调用者；信任声明模式（两者皆无）才放行匿名。
    fn control_principal(&self, headers: &HeaderMap) -> Result<Option<String>, ApiError> {
        if self.auth_tokens.is_empty() && !self.signing_enabled() {
            return Ok(None);
        }
        self.authenticated_principal(headers)
    }

    /// 控制面实例端点：认证 +（ACL 启用时）实例授权。
    /// 返回已认证的操作者（信任声明模式为 `None`），供审计字段使用。
    fn authorize_control_key(
        &self,
        headers: &HeaderMap,
        key: &InstanceKey,
    ) -> Result<Option<String>, ApiError> {
        let principal = self.control_principal(headers)?;
        self.check_control_key(principal.as_deref(), key)?;
        Ok(principal)
    }

    /// 已认证后的实例授权检查（ACL 未启用放行）。
    fn check_control_key(
        &self,
        principal: Option<&str>,
        key: &InstanceKey,
    ) -> Result<(), ApiError> {
        if !self.acl_enabled() {
            return Ok(());
        }
        let Some(p) = principal else {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Unauthorized",
                "ACL 已启用但未配置身份（配置非法）".to_owned(),
            ));
        };
        if !acl_allows(&self.acl, p, &key.domain, &key.instance) {
            return Err(Self::forbidden(&key.domain, &key.instance));
        }
        Ok(())
    }

    /// 控制面域级端点：认证 +（ACL 启用时）域通配授权 `domain/*`。
    fn authorize_control_wildcard(
        &self,
        headers: &HeaderMap,
        domain: &str,
    ) -> Result<Option<String>, ApiError> {
        let principal = self.control_principal(headers)?;
        self.check_control_wildcard(principal.as_deref(), domain)?;
        Ok(principal)
    }

    /// 已认证后的域通配授权检查（ACL 未启用放行）。
    fn check_control_wildcard(
        &self,
        principal: Option<&str>,
        domain: &str,
    ) -> Result<(), ApiError> {
        if !self.acl_enabled() {
            return Ok(());
        }
        let Some(p) = principal else {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "Unauthorized",
                "ACL 已启用但未配置身份（配置非法）".to_owned(),
            ));
        };
        if !acl_allows_wildcard(&self.acl, p, domain) {
            return Err(Self::forbidden(domain, "*"));
        }
        Ok(())
    }
}

/// D35：启动时生成的进程内能力令牌（优先取 32 字节系统随机源）。
fn random_internal_token() -> [u8; 32] {
    use std::io::Read as _;
    let mut out = [0u8; 32];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut out))
        .is_ok()
    {
        return out;
    }
    // 退化路径：两枚 ULID 提供 160 位随机熵。
    out[..16].copy_from_slice(&ulid::Ulid::new().to_bytes());
    out[16..].copy_from_slice(&ulid::Ulid::new().to_bytes());
    out
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
            EngineError::InstanceFrozen { domain, instance } => ApiError {
                status: StatusCode::CONFLICT,
                detail: ErrorDetail {
                    code: "InstanceFrozen".to_owned(),
                    detail: serde_json::json!({ "domain": domain, "instance": instance }),
                    message: format!(
                        "实例 {domain}/{instance} 已冻结（状态交接后只读；请使用目标实例或显式解冻）"
                    ),
                },
            },
            EngineError::HandoffIncompatible { detail } => ApiError {
                status: StatusCode::CONFLICT,
                detail: ErrorDetail {
                    code: "HandoffIncompatible".to_owned(),
                    detail: serde_json::json!({ "detail": detail }),
                    message: format!("状态交接不兼容：{detail}"),
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
        // 状态交接 + 活动实例别名（短路由按 active 解析）
        .route("/domains/{d}/handoff", post(handoff))
        .route("/domains/{d}/active", get(active).put(activate))
        .route("/domains/{d}/instances/{i}/freeze", put(set_freeze))
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
        // D36：RFC 9728 受保护资源元数据（豁免签名）。
        .route(
            "/.well-known/oauth-protected-resource",
            get(oauth_protected_resource),
        )
        .fallback(not_found)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::enforce_identity,
        ))
        .with_state(state)
}

/// D35：在公开路由之上挂载 `/mcp`（`[mcp].enabled = true` 时）。
pub fn router_with_mcp(state: Arc<ServerState>, mcp_enabled: bool) -> Router {
    let base = router(state.clone());
    if !mcp_enabled {
        return base;
    }
    let client = Arc::new(crate::client::Client::local(
        base.clone(),
        state.internal_token(),
    ));
    let mut hosts = vec![
        "localhost".to_owned(),
        "127.0.0.1".to_owned(),
        "::1".to_owned(),
    ];
    if let Some(url) = state.public_url() {
        if let Ok(uri) = url.parse::<http::Uri>() {
            if let Some(a) = uri.authority() {
                hosts.push(a.as_str().to_owned());
            }
        }
    }
    let service = crate::mcp::http_service(client, hosts);
    let mcp =
        Router::new()
            .nest_service("/mcp", service)
            .layer(axum::middleware::from_fn_with_state(
                state,
                crate::middleware::require_mcp_auth,
            ));
    base.merge(mcp)
}

// ============ 运维端点（D25 子项 2：health / ready / metrics，不校验令牌） ============

/// 存活探针（liveness）：进程在服务即 200，不依赖引擎状态。
async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

/// D36：RFC 9728 受保护资源元数据（未配置 OIDC 或 public_url 时为 404）。
async fn oauth_protected_resource(State(st): State<Arc<ServerState>>) -> Response {
    match (st.oidc(), st.public_url()) {
        (Some(oidc), Some(url)) => Json(oidc.metadata(url)).into_response(),
        _ => ApiError::new(
            StatusCode::NOT_FOUND,
            "NotFound",
            "未启用 OIDC 受保护资源元数据".to_owned(),
        )
        .into_response(),
    }
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
    let key = st.resolve_key(&d).await?;
    st.authorize_key(&headers, &key)?;
    let out = st.engine.compact(key, req.keep_last).await?;
    Ok(Json(out))
}

/// `POST /domains/{d}/handoff`：状态交接（冻结源 → 新声明目标 → 可选切别名）。
async fn handoff(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<HandoffReq>,
) -> Result<(StatusCode, Json<pact_engine::views::HandoffOutcome>), ApiError> {
    let principal = st.control_principal(&headers)?;
    st.check_control_key(
        principal.as_deref(),
        &InstanceKey::new(&d, &req.source_instance),
    )?;
    st.check_control_key(
        principal.as_deref(),
        &InstanceKey::new(&d, &req.target_instance),
    )?;
    // 交接会改写域级 `declaration.pact`（影响后续新实例），故额外要求域通配授权。
    st.check_control_wildcard(principal.as_deref(), &d)?;
    let out = st
        .engine
        .handoff_as(
            &d,
            &req.source_instance,
            &req.target_instance,
            &req.declaration,
            req.activate,
            principal.as_deref(),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(out)))
}

/// `GET /domains/{d}/active`：当前活动实例（含控制面审计字段）。
async fn active(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ActiveResp>, ApiError> {
    let principal = st.control_principal(&headers)?;
    let pointer = st.engine.active_pointer(&d).await?;
    let instance = pointer
        .as_ref()
        .map(|p| p.instance.clone())
        .unwrap_or_else(|| DEFAULT_INSTANCE.to_owned());
    st.check_control_key(principal.as_deref(), &InstanceKey::new(&d, &instance))?;
    let (ts, by) = match pointer {
        Some(p) => (p.ts, p.by),
        None => (None, None),
    };
    Ok(Json(ActiveResp {
        domain: d,
        instance,
        ts,
        by,
    }))
}

/// `PUT /domains/{d}/active`：切换活动实例指针（域级权限）。
async fn activate(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<ActivateReq>,
) -> Result<Json<ActiveResp>, ApiError> {
    let principal = st.authorize_control_wildcard(&headers, &d)?;
    let instance = st
        .engine
        .activate_as(&d, &req.instance, principal.as_deref())
        .await?;
    let (ts, by) = match st.engine.active_pointer(&d).await? {
        Some(p) => (p.ts, p.by),
        None => (None, None),
    };
    Ok(Json(ActiveResp {
        domain: d,
        instance,
        ts,
        by,
    }))
}

/// `PUT /domains/{d}/instances/{i}/freeze`：设置/解除实例冻结。
async fn set_freeze(
    State(st): State<Arc<ServerState>>,
    Path((d, i)): Path<(String, String)>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<FreezeReq>,
) -> Result<StatusCode, ApiError> {
    let key = InstanceKey::new(d, i);
    let principal = st.authorize_control_key(&headers, &key)?;
    st.engine
        .set_frozen_as(key, req.frozen, principal.as_deref())
        .await?;
    Ok(StatusCode::NO_CONTENT)
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
            if let Some(active) = &listing.active {
                if !acl_allows(&st.acl, &principal, &listing.name, active) {
                    listing.active = None;
                }
            }
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
    let key = st.resolve_key(&d).await?;
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
    let key = st.resolve_key(&d).await?;
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
    seed_inner(&st, st.resolve_key(&d).await?, headers, req).await
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
    produce_inner(&st, st.resolve_key(&d).await?, headers, req).await
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
    counter_inner(&st, st.resolve_key(&d).await?, headers, q).await
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
    let principal = st.view_principal(&headers)?;
    let type_filter = q.ty.clone();
    let mut view = st.engine.counter(key.clone(), q.ty.map(TypeName)).await?;
    let readable = if view.tokens.iter().any(|token| token.thing_ref.0.is_some()) {
        st.readable_types(&key, principal.as_deref()).await?
    } else {
        BTreeSet::new()
    };
    let audit_types = apply_visibility(&st.visibility, &key, view.tokens.iter_mut(), &readable);
    st.record_view_audit(&key, principal, ViewName::Counter, type_filter, audit_types)
        .await?;
    Ok(Json(view))
}

async fn archive(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
    Query(q): Query<TypeQuery>,
) -> Result<Json<pact_engine::views::ArchiveView>, ApiError> {
    archive_inner(&st, st.resolve_key(&d).await?, headers, q).await
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
    let principal = st.view_principal(&headers)?;
    let type_filter = q.ty.clone();
    let mut view = st.engine.archive(key.clone(), q.ty.map(TypeName)).await?;
    let readable = if view
        .entries
        .iter()
        .any(|entry| entry.token.thing_ref.0.is_some())
    {
        st.readable_types(&key, principal.as_deref()).await?
    } else {
        BTreeSet::new()
    };
    let audit_types = apply_visibility(
        &st.visibility,
        &key,
        view.entries.iter_mut().map(|entry| &mut entry.token),
        &readable,
    );
    st.record_view_audit(&key, principal, ViewName::Archive, type_filter, audit_types)
        .await?;
    Ok(Json(view))
}

/// D42/D43：按 profile 与调用者 gate 过滤票面，并返回按类型聚合的审计计数。
/// token 存在性保持不变；审计只记计数，不记 `thing_ref` 值。
fn apply_visibility<'a, I>(
    profiles: &crate::visibility::VisibilityProfiles,
    key: &InstanceKey,
    tokens: I,
    readable: &BTreeSet<TypeName>,
) -> Vec<pact_store::ViewAuditType>
where
    I: IntoIterator<Item = &'a mut pact_core::Token>,
{
    #[derive(Default)]
    struct Tally {
        tokens: usize,
        refs_visible: usize,
        refs_filtered: usize,
    }

    let mut stats: BTreeMap<String, Tally> = BTreeMap::new();
    for token in tokens {
        let tally = stats.entry(token.ty.0.clone()).or_default();
        tally.tokens += 1;
        if token.thing_ref.0.is_none() {
            continue;
        }
        if ref_is_visible(profiles, key, &token.ty, &token.thing_ref, readable) {
            tally.refs_visible += 1;
        } else {
            tally.refs_filtered += 1;
            token.thing_ref = ThingRef::none();
        }
    }
    stats
        .into_iter()
        .map(|(ty, tally)| pact_store::ViewAuditType {
            ty,
            tokens: tally.tokens,
            refs_visible: tally.refs_visible,
            refs_filtered: tally.refs_filtered,
        })
        .collect()
}

fn ref_is_visible(
    profiles: &crate::visibility::VisibilityProfiles,
    key: &InstanceKey,
    ty: &TypeName,
    thing_ref: &ThingRef,
    readable: &BTreeSet<TypeName>,
) -> bool {
    use crate::visibility::RefExposure;
    match profiles.exposure(
        &key.domain,
        &key.instance,
        ty.0.as_str(),
        thing_ref.0.as_deref(),
    ) {
        RefExposure::Visible => true,
        RefExposure::ReadGate => readable.contains(ty),
        RefExposure::Hidden => false,
    }
}

async fn read_token(
    State(st): State<Arc<ServerState>>,
    Path((d, id)): Path<(String, String)>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<ParticipantReq>,
) -> Result<Json<TokenResp>, ApiError> {
    read_inner(&st, st.resolve_key(&d).await?, id, headers, req).await
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
    consume_inner(&st, st.resolve_key(&d).await?, id, headers, req).await
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
    gates_inner(&st, st.resolve_key(&d).await?, participant, headers).await
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
    enabled_inner(&st, st.resolve_key(&d).await?, headers, q).await
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
    read_audit_inner(&st, st.resolve_key(&d).await?, headers, q).await
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
    status_inner(&st, st.resolve_key(&d).await?, headers).await
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
    events_inner(&st, st.resolve_key(&d).await?, headers).await
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

/// TCP 监听循环（可选）；`tls` 存在时在 accept 后完成 TLS 握手（D34）。
pub async fn serve_tcp(
    app: Router,
    addr: &str,
    tls: Option<tokio_rustls::TlsAcceptor>,
    mut shutdown: watch::Receiver<bool>,
    tracker: ConnTracker,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    loop {
        tokio::select! {
            r = listener.accept() => {
                let (stream, _) = r?;
                match &tls {
                    None => spawn_connection(app.clone(), hyper_util::rt::TokioIo::new(stream), tracker.guard()),
                    Some(acceptor) => {
                        let acceptor = acceptor.clone();
                        let app = app.clone();
                        let guard = tracker.guard();
                        tokio::spawn(async move {
                            match acceptor.accept(stream).await {
                                Ok(tls_stream) => {
                                    serve_connection(app, hyper_util::rt::TokioIo::new(tls_stream), guard).await;
                                }
                                Err(e) => {
                                    // 握手失败（缺客户端证书、协议不符等）只影响该连接。
                                    eprintln!("pactd: TLS 握手失败：{e}");
                                }
                            }
                        });
                    }
                }
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
    tokio::spawn(serve_connection(app, io, guard));
}

async fn serve_connection<I>(app: Router, io: I, guard: ConnGuard)
where
    I: hyper::rt::Read + hyper::rt::Write + Send + Unpin + 'static,
{
    let _guard = guard;
    let svc = hyper_util::service::TowerToHyperService::new(app.into_service());
    let builder =
        hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new());
    // 连接级错误（客户端断开等）不影响 daemon 存活
    let _ = builder.serve_connection(io, svc).await;
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
