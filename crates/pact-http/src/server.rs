//! REST daemon（设计 §9.2）：axum 路由 + unix socket / TCP 双监听。
//!
//! 被拒动作返回 4xx + §6.3 结构化错误体；错误码 = RuleError / Problem 变体名。

use crate::dto::{
    conflicts_detail, problem_detail, rule_error_detail, ComposeReq, DomainInfo, DomainsResp,
    EnabledOption, EnabledResp, EntryResp, ErrorBody, ErrorDetail, GatesResp, LoadDomainReq,
    ParticipantReq, ProduceReq, SeedReq, TokenResp, ValidateReq, ValidateResp,
};
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Path, Query, Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{any, get, post};
use axum::{Json, Router};
use futures::StreamExt;
use pact_core::{ParticipantName, ThingRef, TokenId, TypeName};
use pact_engine::views::StatusView;
use pact_engine::{Engine, EngineError};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::path::Path as FsPath;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;

// ============ 状态与错误 ============

pub struct ServerState {
    engine: Engine,
    /// 参与者 → 令牌；空表 = 信任声明模式（NFR5）。
    auth_tokens: BTreeMap<String, String>,
}

impl ServerState {
    pub fn new(engine: Engine, auth_tokens: BTreeMap<String, String>) -> Arc<Self> {
        Arc::new(Self { engine, auth_tokens })
    }

    /// 参与者身份校验（§9.4：命令进入引擎前完成）。
    /// 令牌表为空 = 信任声明模式，直接放行。
    fn authorize(&self, participant: &str, headers: &HeaderMap) -> Result<(), ApiError> {
        if self.auth_tokens.is_empty() {
            return Ok(());
        }
        let expected = self.auth_tokens.get(participant).ok_or_else(|| {
            ApiError::new(StatusCode::UNAUTHORIZED, "Unauthorized",
                format!("参与者 {participant} 无登记令牌"))
        })?;
        let supplied = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "));
        match supplied {
            Some(t) if ct_eq(t, expected) => Ok(()),
            _ => Err(ApiError::new(StatusCode::UNAUTHORIZED, "Unauthorized",
                format!("参与者 {participant} 令牌缺失或不匹配"))),
        }
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
            EngineError::DomainNotFound { name } => ApiError {
                status: StatusCode::NOT_FOUND,
                detail: ErrorDetail {
                    code: "DomainNotFound".to_owned(),
                    detail: serde_json::json!({ "name": name }),
                    message: format!("域未加载：{name}"),
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
            EngineError::Store(s) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR, "Store", format!("存储失败：{s}")),
            EngineError::Recovery(msg) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR, "Recovery", format!("启动恢复失败：{msg}")),
            EngineError::EngineStopped => ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE, "EngineStopped", "引擎已停止".to_owned()),
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
                StatusCode::BAD_REQUEST, "BadRequest", json_rejection_text(r))),
        }
    }
}

fn json_rejection_text(r: JsonRejection) -> String {
    match r {
        JsonRejection::JsonSyntaxError(e) => format!("JSON 语法错误：{e}"),
        JsonRejection::JsonDataError(e) => format!("JSON 类型不符：{e}"),
        JsonRejection::MissingJsonContentType(_) => "缺少 Content-Type: application/json".to_owned(),
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
        .route("/domains", post(load_domain).get(list_domains))
        .route("/validate", post(validate_declaration))
        .route("/domains/{d}", any(domain_entry))
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
        .fallback(not_found)
        .with_state(state)
}

/// `/domains/{d}` 统一入口：matchit 不支持同段“参数+字面量”，
/// 故 `:analyze` / `:compose`（§9.1）在此按段内冒号分发。
async fn domain_entry(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    req: Request,
) -> axum::response::Response {
    let method = req.method().clone();
    if let Some((name, suffix)) = d.split_once(':') {
        if name.is_empty() {
            return not_found_path(&d);
        }
        match (&method, suffix) {
            (&Method::POST, "analyze") => {
                match analyze_domain(State(st), Path(name.to_owned())).await {
                    Ok(body) => body.into_response(),
                    Err(e) => e.into_response(),
                }
            }
            (&Method::POST, "compose") => {
                match JsonReq::<ComposeReq>::from_request(req, &()).await {
                    Ok(j) => match compose_domain(State(st), Path(name.to_owned()), j).await {
                        Ok(resp) => resp.into_response(),
                        Err(e) => e.into_response(),
                    },
                    Err(e) => e.into_response(),
                }
            }
            (_, "analyze" | "compose") => ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED, "MethodNotAllowed",
                format!("方法 {method} 不适用于 :{suffix}（仅 POST）"),
            )
            .into_response(),
            _ => not_found_path(&d),
        }
    } else {
        match method {
            Method::GET => match domain_info(State(st), Path(d)).await {
                Ok(body) => body.into_response(),
                Err(e) => e.into_response(),
            },
            Method::DELETE => match unload_domain(State(st), Path(d)).await {
                Ok(code) => code.into_response(),
                Err(e) => e.into_response(),
            },
            _ => ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED, "MethodNotAllowed",
                format!("方法 {method} 不适用于 /domains/{{d}}（仅 GET/DELETE）"),
            )
            .into_response(),
        }
    }
}

async fn not_found(req: Request) -> axum::response::Response {
    not_found_path(req.uri().path())
}

fn not_found_path(path: &str) -> axum::response::Response {
    ApiError::new(StatusCode::NOT_FOUND, "NotFound", format!("路径不存在：{path}")).into_response()
}

// ============ 处理器 ============

async fn load_domain(
    State(st): State<Arc<ServerState>>,
    JsonReq(req): JsonReq<LoadDomainReq>,
) -> Result<(StatusCode, Json<pact_engine::views::DomainSummary>), ApiError> {
    let summary = st.engine.load_domain(&req.name, &req.declaration).await?;
    Ok((StatusCode::CREATED, Json(summary)))
}

async fn list_domains(State(st): State<Arc<ServerState>>) -> Result<Json<DomainsResp>, ApiError> {
    Ok(Json(DomainsResp { domains: st.engine.list_domains().await? }))
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
) -> Result<Json<DomainInfo>, ApiError> {
    let s = st.engine.status(&d).await?;
    Ok(Json(DomainInfo {
        name: d,
        counter_size: s.counter_size,
        archive_size: s.archive_size,
        terminated: s.terminated,
    }))
}

async fn unload_domain(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
) -> Result<StatusCode, ApiError> {
    st.engine.unload_domain(&d).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn analyze_domain(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
) -> Result<Json<pact_engine::views::AnalysisReport>, ApiError> {
    Ok(Json(st.engine.analyze_loaded(&d).await?))
}

async fn compose_domain(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    JsonReq(req): JsonReq<ComposeReq>,
) -> Result<(StatusCode, Json<pact_engine::views::DomainSummary>), ApiError> {
    let summary = st.engine.compose_and_load(&d, req.fragments).await?;
    Ok((StatusCode::CREATED, Json(summary)))
}

async fn seed(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<SeedReq>,
) -> Result<(StatusCode, Json<TokenResp>), ApiError> {
    st.authorize(&req.participant, &headers)?;
    let token = st
        .engine
        .seed(&d, ParticipantName(req.participant), TypeName(req.ty), thing_ref(req.thing_ref))
        .await?;
    Ok((StatusCode::CREATED, Json(TokenResp { token })))
}

async fn produce(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<ProduceReq>,
) -> Result<(StatusCode, Json<TokenResp>), ApiError> {
    st.authorize(&req.participant, &headers)?;
    let predecessors = req.predecessors.into_iter().map(TokenId).collect();
    let token = st
        .engine
        .produce(
            &d,
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
    Query(q): Query<TypeQuery>,
) -> Result<Json<pact_engine::views::CounterView>, ApiError> {
    Ok(Json(st.engine.counter(&d, q.ty.map(TypeName)).await?))
}

async fn archive(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    Query(q): Query<TypeQuery>,
) -> Result<Json<pact_engine::views::ArchiveView>, ApiError> {
    Ok(Json(st.engine.archive(&d, q.ty.map(TypeName)).await?))
}

async fn read_token(
    State(st): State<Arc<ServerState>>,
    Path((d, id)): Path<(String, String)>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<ParticipantReq>,
) -> Result<Json<TokenResp>, ApiError> {
    st.authorize(&req.participant, &headers)?;
    let token = st.engine.read(&d, ParticipantName(req.participant), TokenId(id)).await?;
    Ok(Json(TokenResp { token }))
}

async fn consume_token(
    State(st): State<Arc<ServerState>>,
    Path((d, id)): Path<(String, String)>,
    headers: HeaderMap,
    JsonReq(req): JsonReq<ParticipantReq>,
) -> Result<Json<EntryResp>, ApiError> {
    st.authorize(&req.participant, &headers)?;
    let entry = st.engine.consume(&d, ParticipantName(req.participant), TokenId(id)).await?;
    Ok(Json(EntryResp { entry }))
}

async fn gates(
    State(st): State<Arc<ServerState>>,
    Path((d, participant)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<GatesResp>, ApiError> {
    st.authorize(&participant, &headers)?;
    let gates = st.engine.my_gates(&d, ParticipantName(participant)).await?;
    Ok(Json(GatesResp { gates }))
}

async fn enabled(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
    headers: HeaderMap,
    Query(q): Query<ParticipantQuery>,
) -> Result<Json<EnabledResp>, ApiError> {
    st.authorize(&q.participant, &headers)?;
    let options = st
        .engine
        .enabled(&d, ParticipantName(q.participant))
        .await?
        .into_iter()
        .map(EnabledOption::from)
        .collect();
    Ok(Json(EnabledResp { options }))
}

async fn status(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
) -> Result<Json<StatusView>, ApiError> {
    Ok(Json(st.engine.status(&d).await?))
}

/// §6.4 SSE：Counter 变更通知流。
async fn events(
    State(st): State<Arc<ServerState>>,
    Path(d): Path<String>,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let rx = st.engine.subscribe(&d).await?;
    let stream = BroadcastStream::new(rx).filter_map(|r| async move {
        r.ok().map(|n| Ok(Event::default().json_data(n).unwrap_or_else(|_| Event::default())))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn thing_ref(s: Option<String>) -> ThingRef {
    ThingRef(s)
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

// ============ 监听 ============

/// unix socket 监听循环（默认；单机安全）。启动前移除旧套接字文件。
pub async fn serve_unix(app: Router, path: &FsPath) -> std::io::Result<()> {
    let _ = std::fs::remove_file(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    loop {
        let (stream, _) = listener.accept().await?;
        spawn_connection(app.clone(), hyper_util::rt::TokioIo::new(stream));
    }
}

/// TCP 监听循环（可选，建议 127.0.0.1）。
pub async fn serve_tcp(app: Router, addr: &str) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        spawn_connection(app.clone(), hyper_util::rt::TokioIo::new(stream));
    }
}

/// 单连接服务（hyper-util auto：HTTP/1.1）。
fn spawn_connection<I>(app: Router, io: I)
where
    I: hyper::rt::Read + hyper::rt::Write + Send + Unpin + 'static,
{
    tokio::spawn(async move {
        let svc = hyper_util::service::TowerToHyperService::new(app.into_service());
        let builder = hyper_util::server::conn::auto::Builder::new(
            hyper_util::rt::TokioExecutor::new(),
        );
        // 连接级错误（客户端断开等）不影响 daemon 存活
        let _ = builder.serve_connection(io, svc).await;
    });
}
