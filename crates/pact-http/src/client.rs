//! 类型化 client（CLI / MCP 共用）：默认 unix socket，TCP 可选。
//!
//! 响应体统一为 `hyper::body::Incoming`；SSE（§6.4）由 [`EventStream`]
//! 手动解析 `data:` 行为 [`Notification`]。

use crate::dto::{
    CompactReq, ComposeReq, ConsumeReq, DomainInfo, DomainsResp, EnabledResp, EntryResp, ErrorBody,
    ErrorDetail, GatesResp, InstantiateReq, LoadDomainReq, ParticipantReq, ProduceReq,
    ReadAuditResp, SeedReq, TokenResp, ValidateReq, ValidateResp,
};
use crate::sign::SigningIdentity;
use axum::Router;
use bytes::Bytes;
use http::uri::Uri;
use http::{Method, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo};
use pact_engine::notify::Notification;
use pact_engine::views::{
    AnalysisReport, ArchiveView, CompactionOutcome, CounterView, DomainSummary, StatusView,
};
use pact_engine::DEFAULT_INSTANCE;
use serde::de::DeserializeOwned;
use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::ServiceExt;
use tower_service::Service;

type Hyper<C> = hyper_util::client::legacy::Client<C, Full<Bytes>>;

/// 域/实例 → 路径基：`None` 或 `default` ⇒ 默认实例路径族；
/// 其余 ⇒ 实例级镜像组（D27）。
fn base_path(domain: &str, instance: Option<&str>) -> String {
    match instance {
        None | Some(DEFAULT_INSTANCE) => format!("/domains/{domain}"),
        Some(i) => format!("/domains/{domain}/instances/{i}"),
    }
}

/// daemon 错误（§6.3 结构化体）。
#[derive(Debug)]
pub enum ClientError {
    Io(std::io::Error),
    /// 连接/传输层失败。
    Http(Box<dyn std::error::Error + Send + Sync>),
    /// daemon 以 4xx/5xx + 结构化错误体拒绝。
    Api(ErrorDetail),
    /// 响应体不是合法 §6 线上格式。
    Parse(serde_json::Error),
    /// 非预期的状态码（且无法解析为 §6.3 错误体）。
    UnexpectedStatus(StatusCode),
    /// 客户端配置非法（如 HTTPS 缺地址、签名身份与参与者不匹配）。
    Config(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Io(e) => write!(f, "IO 错误：{e}"),
            ClientError::Http(e) => write!(f, "传输错误：{e}"),
            ClientError::Api(d) => write!(f, "[{}] {}", d.code, d.message),
            ClientError::Parse(e) => write!(f, "响应解析失败：{e}"),
            ClientError::UnexpectedStatus(s) => write!(f, "非预期状态码：{s}"),
            ClientError::Config(msg) => write!(f, "客户端配置错误：{msg}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// unix socket 连接器（忽略 Uri 主机，直连本地套接字）。
#[derive(Clone)]
struct UnixConnector {
    path: PathBuf,
}

impl Service<Uri> for UnixConnector {
    type Response = TokioIo<tokio::net::UnixStream>;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: Uri) -> Self::Future {
        let path = self.path.clone();
        Box::pin(async move {
            tokio::net::UnixStream::connect(path)
                .await
                .map(TokioIo::new)
        })
    }
}

#[derive(Clone)]
enum Inner {
    Unix(Hyper<UnixConnector>),
    Tcp(Hyper<HttpConnector>),
    Tls(Hyper<hyper_rustls::HttpsConnector<HttpConnector>>),
    /// D35：daemon 内建 MCP 的进程内路由（无网络自连接）。
    Local(Router),
}

/// PACT daemon client。
#[derive(Clone)]
pub struct Client {
    inner: Inner,
    /// 请求 Uri 的 authority 段（unix 下仅占位）。
    authority: String,
    /// 请求 scheme（D34：`https` 走 TLS）。
    scheme: &'static str,
    /// §9.4：非空令牌表时随请求附 `Authorization: Bearer`。
    bearer: Option<String>,
    /// D34：Ed25519 请求签名身份（可选）。
    signing: Option<SigningIdentity>,
    /// D35：本地路由的进程内能力令牌。
    local_internal: Option<[u8; 32]>,
    /// D35：本地调用使用的已验证参与者。
    verified_principal: Option<String>,
}

impl Client {
    /// 连接 unix socket daemon（默认）。
    pub fn unix(path: impl Into<PathBuf>) -> Self {
        let conn = UnixConnector { path: path.into() };
        let client: Hyper<UnixConnector> =
            hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build(conn);
        Self {
            inner: Inner::Unix(client),
            authority: "pactd".to_owned(),
            scheme: "http",
            bearer: None,
            signing: None,
            local_internal: None,
            verified_principal: None,
        }
    }

    /// 连接 TCP daemon（如 "127.0.0.1:8787"）。
    pub fn tcp(addr: impl Into<String>) -> Self {
        let client: Hyper<HttpConnector> =
            hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build_http();
        Self {
            inner: Inner::Tcp(client),
            authority: addr.into(),
            scheme: "http",
            bearer: None,
            signing: None,
            local_internal: None,
            verified_principal: None,
        }
    }

    /// D34：连接 HTTPS daemon；`ca` 存在时仅信任该 CA，否则使用 webpki 根证书。
    pub fn https(addr: impl Into<String>, ca: Option<PathBuf>) -> Result<Self, ClientError> {
        let tls =
            crate::tls::client_config(ca.as_deref()).map_err(|e| ClientError::Http(Box::new(e)))?;
        let conn = hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(tls)
            .https_or_http()
            .enable_http1()
            .build();
        let client: Hyper<hyper_rustls::HttpsConnector<HttpConnector>> =
            hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build(conn);
        Ok(Self {
            inner: Inner::Tls(client),
            authority: addr.into(),
            scheme: "https",
            bearer: None,
            signing: None,
            local_internal: None,
            verified_principal: None,
        })
    }

    /// D35：进程内路由（daemon 内建 MCP 调用自身）。
    pub fn local(app: Router, internal_token: [u8; 32]) -> Self {
        Self {
            inner: Inner::Local(app),
            authority: "pactd.local".to_owned(),
            scheme: "http",
            bearer: None,
            signing: None,
            local_internal: Some(internal_token),
            verified_principal: None,
        }
    }

    /// 设置参与者令牌（信任声明模式下可省略）。
    pub fn with_bearer(mut self, token: impl Into<String>) -> Self {
        self.bearer = Some(token.into());
        self
    }

    /// D34：设置 Ed25519 请求签名身份。
    pub fn with_signing(mut self, id: SigningIdentity) -> Self {
        self.signing = Some(id);
        self
    }

    /// D35：本地调用时携带已验证参与者（仅本地路由生效）。
    pub fn with_verified_principal(mut self, principal: Option<String>) -> Self {
        self.verified_principal = principal;
        self
    }

    pub fn is_local(&self) -> bool {
        matches!(self.inner, Inner::Local(_))
    }

    // —— 域生命周期 ——

    /// `POST /domains`：声明 + 实例化（`None` ⇒ 默认实例）。
    pub async fn load_domain(
        &self,
        name: &str,
        declaration: &str,
        instance: Option<&str>,
        force: bool,
    ) -> Result<DomainSummary, ClientError> {
        let req = self.json_request(
            Method::POST,
            "/domains",
            Some(&LoadDomainReq {
                name: name.to_owned(),
                declaration: declaration.to_owned(),
                instance: instance.map(str::to_owned),
                force: Some(force),
            }),
        )?;
        decode(self.dispatch(req).await?).await
    }

    /// `POST /domains/{d}/instances`：在已登记声明上创建额外实例。
    pub async fn instantiate(
        &self,
        domain: &str,
        instance: &str,
    ) -> Result<DomainSummary, ClientError> {
        let req = self.json_request(
            Method::POST,
            &format!("/domains/{domain}/instances"),
            Some(&InstantiateReq {
                instance: instance.to_owned(),
            }),
        )?;
        decode(self.dispatch(req).await?).await
    }

    /// `POST /validate`：校验声明文本良构（不加载）。
    pub async fn validate(&self, declaration: &str) -> Result<ValidateResp, ClientError> {
        let req = self.json_request(
            Method::POST,
            "/validate",
            Some(&ValidateReq {
                declaration: declaration.to_owned(),
            }),
        )?;
        decode(self.dispatch(req).await?).await
    }

    pub async fn list_domains(&self) -> Result<DomainsResp, ClientError> {
        let req = self.json_request::<()>(Method::GET, "/domains", None)?;
        decode(self.dispatch(req).await?).await
    }

    pub async fn domain_info(
        &self,
        domain: &str,
        instance: Option<&str>,
    ) -> Result<DomainInfo, ClientError> {
        let req = self.json_request::<()>(Method::GET, &base_path(domain, instance), None)?;
        decode(self.dispatch(req).await?).await
    }

    /// `DELETE /domains/{d}[/instances/{i}]`（204）。
    pub async fn unload_domain(
        &self,
        domain: &str,
        instance: Option<&str>,
    ) -> Result<(), ClientError> {
        let req = self.json_request::<()>(Method::DELETE, &base_path(domain, instance), None)?;
        expect_empty(self.dispatch(req).await?).await
    }

    /// `POST /domains/{d}[/instances/{i}]/compact`：日志前缀压缩（D25 子项 1）。
    /// `keep_last` = 保留最近的事件条数（0 = 丢弃全部已入快照的前缀）。
    pub async fn compact(
        &self,
        domain: &str,
        instance: Option<&str>,
        keep_last: u64,
    ) -> Result<CompactionOutcome, ClientError> {
        let req = self.json_request(
            Method::POST,
            &format!("{}/compact", base_path(domain, instance)),
            Some(&CompactReq { keep_last }),
        )?;
        decode(self.dispatch(req).await?).await
    }

    /// `POST /domains/{d}:analyze`（定义级，无实例维度）。
    pub async fn analyze_domain(&self, domain: &str) -> Result<AnalysisReport, ClientError> {
        let req =
            self.json_request::<()>(Method::POST, &format!("/domains/{domain}:analyze"), None)?;
        decode(self.dispatch(req).await?).await
    }

    /// `POST /domains/{d}:compose`（作用于默认实例）。
    pub async fn compose(
        &self,
        name: &str,
        fragments: Vec<String>,
    ) -> Result<DomainSummary, ClientError> {
        let req = self.json_request(
            Method::POST,
            &format!("/domains/{name}:compose"),
            Some(&ComposeReq { fragments }),
        )?;
        decode(self.dispatch(req).await?).await
    }

    // —— 四类动作（Rule-S/P/R/E）——

    pub async fn seed(
        &self,
        domain: &str,
        instance: Option<&str>,
        participant: &str,
        ty: &str,
        thing_ref: Option<&str>,
    ) -> Result<TokenResp, ClientError> {
        let req = self.json_request(
            Method::POST,
            &format!("{}/seeds", base_path(domain, instance)),
            Some(&SeedReq {
                participant: participant.to_owned(),
                ty: ty.to_owned(),
                thing_ref: thing_ref.map(str::to_owned),
            }),
        )?;
        decode(self.dispatch(req).await?).await
    }

    pub async fn produce(
        &self,
        domain: &str,
        instance: Option<&str>,
        participant: &str,
        ty: &str,
        predecessors: Vec<String>,
        thing_ref: Option<&str>,
    ) -> Result<TokenResp, ClientError> {
        let req = self.json_request(
            Method::POST,
            &format!("{}/tokens", base_path(domain, instance)),
            Some(&ProduceReq {
                participant: participant.to_owned(),
                ty: ty.to_owned(),
                predecessors,
                thing_ref: thing_ref.map(str::to_owned),
            }),
        )?;
        decode(self.dispatch(req).await?).await
    }

    pub async fn read_token(
        &self,
        domain: &str,
        instance: Option<&str>,
        participant: &str,
        token_id: &str,
    ) -> Result<TokenResp, ClientError> {
        let req = self.json_request(
            Method::POST,
            &format!("{}/tokens/{token_id}/read", base_path(domain, instance)),
            Some(&ParticipantReq {
                participant: participant.to_owned(),
            }),
        )?;
        decode(self.dispatch(req).await?).await
    }

    pub async fn consume(
        &self,
        domain: &str,
        instance: Option<&str>,
        participant: &str,
        token_id: &str,
        request_id: Option<&str>,
    ) -> Result<EntryResp, ClientError> {
        let req = self.json_request(
            Method::POST,
            &format!("{}/tokens/{token_id}/consume", base_path(domain, instance)),
            Some(&ConsumeReq {
                participant: participant.to_owned(),
                request_id: request_id.map(str::to_owned),
            }),
        )?;
        decode(self.dispatch(req).await?).await
    }

    // —— 查询 ——

    pub async fn counter(
        &self,
        domain: &str,
        instance: Option<&str>,
        ty: Option<&str>,
    ) -> Result<CounterView, ClientError> {
        let base = base_path(domain, instance);
        let path = match ty {
            Some(t) => format!("{base}/counter?type={t}"),
            None => format!("{base}/counter"),
        };
        let req = self.json_request::<()>(Method::GET, &path, None)?;
        decode(self.dispatch(req).await?).await
    }

    pub async fn archive(
        &self,
        domain: &str,
        instance: Option<&str>,
        ty: Option<&str>,
    ) -> Result<ArchiveView, ClientError> {
        let base = base_path(domain, instance);
        let path = match ty {
            Some(t) => format!("{base}/archive?type={t}"),
            None => format!("{base}/archive"),
        };
        let req = self.json_request::<()>(Method::GET, &path, None)?;
        decode(self.dispatch(req).await?).await
    }

    pub async fn gates(
        &self,
        domain: &str,
        instance: Option<&str>,
        participant: &str,
    ) -> Result<GatesResp, ClientError> {
        let req = self.json_request::<()>(
            Method::GET,
            &format!("{}/gates/{participant}", base_path(domain, instance)),
            None,
        )?;
        decode(self.dispatch(req).await?).await
    }

    pub async fn enabled(
        &self,
        domain: &str,
        instance: Option<&str>,
        participant: &str,
    ) -> Result<EnabledResp, ClientError> {
        let req = self.json_request::<()>(
            Method::GET,
            &format!(
                "{}/enabled?participant={participant}",
                base_path(domain, instance)
            ),
            None,
        )?;
        decode(self.dispatch(req).await?).await
    }

    pub async fn status(
        &self,
        domain: &str,
        instance: Option<&str>,
    ) -> Result<StatusView, ClientError> {
        let req = self.json_request::<()>(
            Method::GET,
            &format!("{}/status", base_path(domain, instance)),
            None,
        )?;
        decode(self.dispatch(req).await?).await
    }

    /// 读审计只读视图（D25 子项 3）：最近 `limit` 条 Rule-R 记录（时间正序）。
    pub async fn read_audit(
        &self,
        domain: &str,
        instance: Option<&str>,
        limit: Option<usize>,
    ) -> Result<ReadAuditResp, ClientError> {
        let path = match limit {
            Some(n) => format!("{}/read-audit?limit={n}", base_path(domain, instance)),
            None => format!("{}/read-audit", base_path(domain, instance)),
        };
        let req = self.json_request::<()>(Method::GET, &path, None)?;
        decode(self.dispatch(req).await?).await
    }

    /// §6.4 SSE：订阅实例的 Counter 变更通知流。
    pub async fn events(
        &self,
        domain: &str,
        instance: Option<&str>,
    ) -> Result<EventStream, ClientError> {
        let req = self.json_request::<()>(
            Method::GET,
            &format!("{}/events", base_path(domain, instance)),
            None,
        )?;
        let (status, body) = self.dispatch_stream(req).await?;
        if !status.is_success() {
            let body = match body {
                BodySource::Http(b) => b.collect().await.map_err(boxed)?.to_bytes(),
                BodySource::Local(b) => axum::body::to_bytes(b, crate::sign::MAX_SIGNATURE_BODY)
                    .await
                    .map_err(boxed)?,
            };
            return Err(error_from_body(RawResponse { status, body }));
        }
        Ok(EventStream::new(body))
    }

    // —— 内部 ——

    fn json_request<T: serde::Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&T>,
    ) -> Result<Request<Full<Bytes>>, ClientError> {
        let uri: Uri = format!("{}://{}{}", self.scheme, self.authority, path)
            .parse()
            .map_err(|e: http::uri::InvalidUri| ClientError::Http(Box::new(e)))?;
        let path_and_query = uri
            .path_and_query()
            .map(|p| p.as_str())
            .unwrap_or("/")
            .to_owned();
        let bytes = match body {
            Some(v) => Bytes::from(serde_json::to_vec(v).map_err(ClientError::Parse)?),
            None => Bytes::new(),
        };

        let mut builder = Request::builder().method(method.clone()).uri(uri);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        if let Some(t) = &self.bearer {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        if let Some(id) = &self.signing {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| ClientError::Config(format!("系统时钟早于 Unix 纪元：{e}")))?
                .as_secs();
            let nonce = ulid::Ulid::new().to_string();
            let h = id.headers(method.as_str(), &path_and_query, &bytes, timestamp, &nonce);
            builder = builder
                .header(crate::sign::HEADER_PARTICIPANT, h.participant)
                .header(crate::sign::HEADER_TIMESTAMP, h.timestamp.to_string())
                .header(crate::sign::HEADER_NONCE, h.nonce)
                .header(crate::sign::HEADER_SIGNATURE, h.signature);
        }
        if let Some(token) = &self.local_internal {
            builder = builder.header(crate::sign::HEADER_INTERNAL_AUTH, hex_encode(token));
        }
        if let Some(p) = &self.verified_principal {
            builder = builder.header(crate::sign::HEADER_VERIFIED_PRINCIPAL, p);
        }
        builder
            .body(Full::new(bytes))
            .map_err(|e| ClientError::Http(Box::new(e)))
    }

    /// 聚合响应（本地路由与网络路径统一为 `RawResponse`）。
    async fn dispatch(&self, req: Request<Full<Bytes>>) -> Result<RawResponse, ClientError> {
        match &self.inner {
            Inner::Local(app) => {
                let resp = app
                    .clone()
                    .oneshot(req.map(axum::body::Body::new))
                    .await
                    .map_err(|e| ClientError::Http(Box::new(e)))?;
                let (parts, body) = resp.into_parts();
                let body = axum::body::to_bytes(body, crate::sign::MAX_SIGNATURE_BODY)
                    .await
                    .map_err(|e| ClientError::Http(Box::new(e)))?;
                Ok(RawResponse {
                    status: parts.status,
                    body,
                })
            }
            Inner::Unix(c) => collect(c.request(req).await.map_err(boxed)?).await,
            Inner::Tcp(c) => collect(c.request(req).await.map_err(boxed)?).await,
            Inner::Tls(c) => collect(c.request(req).await.map_err(boxed)?).await,
        }
    }

    /// 流式响应（SSE 订阅）：本地路由保留 axum Body，网络路径保留 Incoming。
    async fn dispatch_stream(
        &self,
        req: Request<Full<Bytes>>,
    ) -> Result<(StatusCode, BodySource), ClientError> {
        if let Inner::Local(app) = &self.inner {
            let resp = app
                .clone()
                .oneshot(req.map(axum::body::Body::new))
                .await
                .map_err(|e| ClientError::Http(Box::new(e)))?;
            let (parts, body) = resp.into_parts();
            return Ok((parts.status, BodySource::Local(body)));
        }
        let resp = match &self.inner {
            Inner::Unix(c) => c.request(req).await.map_err(boxed)?,
            Inner::Tcp(c) => c.request(req).await.map_err(boxed)?,
            Inner::Tls(c) => c.request(req).await.map_err(boxed)?,
            Inner::Local(_) => unreachable!(),
        };
        let (parts, body) = resp.into_parts();
        Ok((parts.status, BodySource::Http(body)))
    }
}

fn boxed<E: std::error::Error + Send + Sync + 'static>(e: E) -> ClientError {
    ClientError::Http(Box::new(e))
}

/// 聚合响应。
struct RawResponse {
    status: StatusCode,
    body: Bytes,
}

async fn collect(resp: hyper::Response<Incoming>) -> Result<RawResponse, ClientError> {
    let (parts, body) = resp.into_parts();
    let body = body.collect().await.map_err(boxed)?.to_bytes();
    Ok(RawResponse {
        status: parts.status,
        body,
    })
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// 2xx → JSON 反序列化；否则解析 §6.3 错误体。
async fn decode<T: DeserializeOwned>(resp: RawResponse) -> Result<T, ClientError> {
    if resp.status.is_success() {
        return serde_json::from_slice(&resp.body).map_err(ClientError::Parse);
    }
    match serde_json::from_slice::<ErrorBody>(&resp.body) {
        Ok(b) => Err(ClientError::Api(b.error)),
        Err(_) => Err(ClientError::UnexpectedStatus(resp.status)),
    }
}

/// 期望空体成功（204）。
async fn expect_empty(resp: RawResponse) -> Result<(), ClientError> {
    if resp.status.is_success() {
        return Ok(());
    }
    Err(error_from_body(resp))
}

fn error_from_body(resp: RawResponse) -> ClientError {
    match serde_json::from_slice::<ErrorBody>(&resp.body) {
        Ok(b) => ClientError::Api(b.error),
        Err(_) => ClientError::UnexpectedStatus(resp.status),
    }
}

/// 订阅流的响应体来源（网络 / 本地路由）。
enum BodySource {
    Http(Incoming),
    Local(axum::body::Body),
}

/// §6.4 SSE 事件流：缓冲帧、按空行切事件、解析 `data:` 行。
pub struct EventStream {
    body: BodySource,
    buf: Vec<u8>,
    queue: VecDeque<Notification>,
}

impl EventStream {
    fn new(body: BodySource) -> Self {
        Self {
            body,
            buf: Vec::new(),
            queue: VecDeque::new(),
        }
    }

    /// 下一通知；`None` = 服务端关闭了流。
    pub async fn next_event(&mut self) -> Result<Option<Notification>, ClientError> {
        loop {
            if let Some(n) = self.queue.pop_front() {
                return Ok(Some(n));
            }
            let chunk = match &mut self.body {
                BodySource::Http(body) => match body.frame().await {
                    Some(Ok(frame)) => frame.data_ref().map(|d| Bytes::copy_from_slice(d)),
                    Some(Err(e)) => return Err(ClientError::Http(Box::new(e))),
                    None => None,
                },
                BodySource::Local(body) => match body.frame().await {
                    Some(Ok(frame)) => frame.data_ref().map(|d| Bytes::copy_from_slice(d)),
                    Some(Err(e)) => return Err(ClientError::Http(Box::new(e))),
                    None => None,
                },
            };
            match chunk {
                Some(chunk) => {
                    self.buf.extend_from_slice(&chunk);
                    self.drain_events();
                }
                None => {
                    self.drain_events();
                    return Ok(self.queue.pop_front());
                }
            }
        }
    }

    /// 从缓冲中切出完整事件（以空行 `\n\n` 结尾）。
    fn drain_events(&mut self) {
        while let Some(pos) = self.buf.windows(2).position(|w| w == b"\n\n") {
            let event: Vec<u8> = self.buf.drain(..pos + 2).collect();
            let mut data_lines: Vec<&str> = Vec::new();
            let text = String::from_utf8_lossy(&event);
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("data:") {
                    data_lines.push(rest.trim_start_matches(' '));
                }
            }
            if data_lines.is_empty() {
                continue; // keep-alive 注释等
            }
            if let Ok(n) = serde_json::from_str::<Notification>(&data_lines.join("\n")) {
                self.queue.push_back(n);
            }
        }
    }
}
