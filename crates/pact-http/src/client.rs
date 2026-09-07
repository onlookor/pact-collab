//! 类型化 client（CLI / MCP 共用）：默认 unix socket，TCP 可选。
//!
//! 响应体统一为 `hyper::body::Incoming`；SSE（§6.4）由 [`EventStream`]
//! 手动解析 `data:` 行为 [`Notification`]。

use crate::dto::{
    ComposeReq, DomainInfo, DomainsResp, EnabledResp, EntryResp, ErrorBody, ErrorDetail, GatesResp,
    LoadDomainReq, ParticipantReq, ProduceReq, SeedReq, TokenResp, ValidateReq, ValidateResp,
};
use bytes::Bytes;
use http::uri::Uri;
use http::{Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo};
use pact_engine::notify::Notification;
use pact_engine::views::{AnalysisReport, ArchiveView, CounterView, DomainSummary, StatusView};
use serde::de::DeserializeOwned;
use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower_service::Service;

type Hyper<C> = hyper_util::client::legacy::Client<C, Full<Bytes>>;

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
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Io(e) => write!(f, "IO 错误：{e}"),
            ClientError::Http(e) => write!(f, "传输错误：{e}"),
            ClientError::Api(d) => write!(f, "[{}] {}", d.code, d.message),
            ClientError::Parse(e) => write!(f, "响应解析失败：{e}"),
            ClientError::UnexpectedStatus(s) => write!(f, "非预期状态码：{s}"),
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

enum Inner {
    Unix(Hyper<UnixConnector>),
    Tcp(Hyper<HttpConnector>),
}

/// PACT daemon client。
pub struct Client {
    inner: Inner,
    /// 请求 Uri 的 authority 段（unix 下仅占位）。
    authority: String,
    /// §9.4：非空令牌表时随请求附 `Authorization: Bearer`。
    bearer: Option<String>,
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
            bearer: None,
        }
    }

    /// 连接 TCP daemon（如 "127.0.0.1:8787"）。
    pub fn tcp(addr: impl Into<String>) -> Self {
        let client: Hyper<HttpConnector> =
            hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build_http();
        Self {
            inner: Inner::Tcp(client),
            authority: addr.into(),
            bearer: None,
        }
    }

    /// 设置参与者令牌（信任声明模式下可省略）。
    pub fn with_bearer(mut self, token: impl Into<String>) -> Self {
        self.bearer = Some(token.into());
        self
    }

    // —— 域生命周期 ——

    pub async fn load_domain(
        &self,
        name: &str,
        declaration: &str,
    ) -> Result<DomainSummary, ClientError> {
        let req = self.json_request(
            Method::POST,
            "/domains",
            Some(&LoadDomainReq {
                name: name.to_owned(),
                declaration: declaration.to_owned(),
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

    pub async fn domain_info(&self, domain: &str) -> Result<DomainInfo, ClientError> {
        let req = self.json_request::<()>(Method::GET, &format!("/domains/{domain}"), None)?;
        decode(self.dispatch(req).await?).await
    }

    /// `DELETE /domains/{d}`（204）。
    pub async fn unload_domain(&self, domain: &str) -> Result<(), ClientError> {
        let req = self.json_request::<()>(Method::DELETE, &format!("/domains/{domain}"), None)?;
        expect_empty(self.dispatch(req).await?).await
    }

    pub async fn analyze_domain(&self, domain: &str) -> Result<AnalysisReport, ClientError> {
        let req =
            self.json_request::<()>(Method::POST, &format!("/domains/{domain}:analyze"), None)?;
        decode(self.dispatch(req).await?).await
    }

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
        participant: &str,
        ty: &str,
        thing_ref: Option<&str>,
    ) -> Result<TokenResp, ClientError> {
        let req = self.json_request(
            Method::POST,
            &format!("/domains/{domain}/seeds"),
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
        participant: &str,
        ty: &str,
        predecessors: Vec<String>,
        thing_ref: Option<&str>,
    ) -> Result<TokenResp, ClientError> {
        let req = self.json_request(
            Method::POST,
            &format!("/domains/{domain}/tokens"),
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
        participant: &str,
        token_id: &str,
    ) -> Result<TokenResp, ClientError> {
        let req = self.json_request(
            Method::POST,
            &format!("/domains/{domain}/tokens/{token_id}/read"),
            Some(&ParticipantReq {
                participant: participant.to_owned(),
            }),
        )?;
        decode(self.dispatch(req).await?).await
    }

    pub async fn consume(
        &self,
        domain: &str,
        participant: &str,
        token_id: &str,
    ) -> Result<EntryResp, ClientError> {
        let req = self.json_request(
            Method::POST,
            &format!("/domains/{domain}/tokens/{token_id}/consume"),
            Some(&ParticipantReq {
                participant: participant.to_owned(),
            }),
        )?;
        decode(self.dispatch(req).await?).await
    }

    // —— 查询 ——

    pub async fn counter(
        &self,
        domain: &str,
        ty: Option<&str>,
    ) -> Result<CounterView, ClientError> {
        let path = match ty {
            Some(t) => format!("/domains/{domain}/counter?type={t}"),
            None => format!("/domains/{domain}/counter"),
        };
        let req = self.json_request::<()>(Method::GET, &path, None)?;
        decode(self.dispatch(req).await?).await
    }

    pub async fn archive(
        &self,
        domain: &str,
        ty: Option<&str>,
    ) -> Result<ArchiveView, ClientError> {
        let path = match ty {
            Some(t) => format!("/domains/{domain}/archive?type={t}"),
            None => format!("/domains/{domain}/archive"),
        };
        let req = self.json_request::<()>(Method::GET, &path, None)?;
        decode(self.dispatch(req).await?).await
    }

    pub async fn gates(&self, domain: &str, participant: &str) -> Result<GatesResp, ClientError> {
        let req = self.json_request::<()>(
            Method::GET,
            &format!("/domains/{domain}/gates/{participant}"),
            None,
        )?;
        decode(self.dispatch(req).await?).await
    }

    pub async fn enabled(
        &self,
        domain: &str,
        participant: &str,
    ) -> Result<EnabledResp, ClientError> {
        let req = self.json_request::<()>(
            Method::GET,
            &format!("/domains/{domain}/enabled?participant={participant}"),
            None,
        )?;
        decode(self.dispatch(req).await?).await
    }

    pub async fn status(&self, domain: &str) -> Result<StatusView, ClientError> {
        let req =
            self.json_request::<()>(Method::GET, &format!("/domains/{domain}/status"), None)?;
        decode(self.dispatch(req).await?).await
    }

    /// §6.4 SSE：订阅 Counter 变更通知流。
    pub async fn events(&self, domain: &str) -> Result<EventStream, ClientError> {
        let req =
            self.json_request::<()>(Method::GET, &format!("/domains/{domain}/events"), None)?;
        let resp = self.dispatch(req).await?;
        let (parts, body) = resp.into_parts();
        if !parts.status.is_success() {
            return Err(error_from_body(body).await);
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
        let uri: Uri = format!("http://{}{}", self.authority, path)
            .parse()
            .map_err(|e: http::uri::InvalidUri| ClientError::Http(Box::new(e)))?;
        let mut builder = Request::builder().method(method).uri(uri);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        if let Some(t) = &self.bearer {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        let bytes = match body {
            Some(v) => Bytes::from(serde_json::to_vec(v).map_err(ClientError::Parse)?),
            None => Bytes::new(),
        };
        builder
            .body(Full::new(bytes))
            .map_err(|e| ClientError::Http(Box::new(e)))
    }

    async fn dispatch(&self, req: Request<Full<Bytes>>) -> Result<Response<Incoming>, ClientError> {
        match &self.inner {
            Inner::Unix(c) => c
                .request(req)
                .await
                .map_err(|e| ClientError::Http(Box::new(e))),
            Inner::Tcp(c) => c
                .request(req)
                .await
                .map_err(|e| ClientError::Http(Box::new(e))),
        }
    }
}

/// 2xx → JSON 反序列化；否则解析 §6.3 错误体。
async fn decode<T: DeserializeOwned>(resp: Response<Incoming>) -> Result<T, ClientError> {
    let (parts, body) = resp.into_parts();
    let bytes = body
        .collect()
        .await
        .map_err(|e| ClientError::Http(Box::new(e)))?
        .to_bytes();
    if parts.status.is_success() {
        return serde_json::from_slice(&bytes).map_err(ClientError::Parse);
    }
    match serde_json::from_slice::<ErrorBody>(&bytes) {
        Ok(b) => Err(ClientError::Api(b.error)),
        Err(_) => Err(ClientError::UnexpectedStatus(parts.status)),
    }
}

/// 期望空体成功（204）。
async fn expect_empty(resp: Response<Incoming>) -> Result<(), ClientError> {
    let (parts, body) = resp.into_parts();
    if parts.status.is_success() {
        return Ok(());
    }
    Err(error_from_body(body).await)
}

async fn error_from_body(body: Incoming) -> ClientError {
    match body.collect().await {
        Ok(collected) => match serde_json::from_slice::<ErrorBody>(&collected.to_bytes()) {
            Ok(b) => ClientError::Api(b.error),
            Err(_) => ClientError::UnexpectedStatus(StatusCode::INTERNAL_SERVER_ERROR),
        },
        Err(e) => ClientError::Http(Box::new(e)),
    }
}

/// §6.4 SSE 事件流：缓冲帧、按空行切事件、解析 `data:` 行。
pub struct EventStream {
    body: Incoming,
    buf: Vec<u8>,
    queue: VecDeque<Notification>,
}

impl EventStream {
    fn new(body: Incoming) -> Self {
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
            match self.body.frame().await {
                Some(Ok(frame)) => {
                    if let Some(chunk) = frame.data_ref() {
                        self.buf.extend_from_slice(chunk);
                        self.drain_events();
                    }
                }
                Some(Err(e)) => return Err(ClientError::Http(Box::new(e))),
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
