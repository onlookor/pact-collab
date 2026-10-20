//! REST 参与者客户端：包装 [`pact_http::client::Client`]（REST over unix socket / TCP）。

use crate::client::{Endpoint, ParticipantClient, WakeStream};
use crate::error::RuntimeError;
use async_trait::async_trait;
use pact_core::{ArchiveEntry, Token};
use pact_engine::views::{CounterView, DomainListing, GateClause, StatusView};
use pact_http::client::Client;
use pact_http::dto::EnabledOption;
use std::path::PathBuf;
use tokio::sync::mpsc;

/// 经 REST 接入 daemon 的参与者客户端。
pub struct RestClient {
    client: Client,
    domain: String,
    /// 实例键；`None` = `default` 实例（D27）。
    instance: Option<String>,
}

impl RestClient {
    /// 以既有 REST 客户端与绑定域构造（作用于 `default` 实例）。
    pub fn new(client: Client, domain: impl Into<String>) -> Self {
        Self {
            client,
            domain: domain.into(),
            instance: None,
        }
    }

    /// 经 unix socket 接入。
    pub fn unix(path: impl Into<PathBuf>, domain: impl Into<String>) -> Self {
        Self::new(Client::unix(path), domain)
    }

    /// 经 TCP 接入。
    pub fn tcp(addr: impl Into<String>, domain: impl Into<String>) -> Self {
        Self::new(Client::tcp(addr), domain)
    }

    /// 按端点与可选 bearer 构造。
    pub fn from_endpoint(endpoint: &Endpoint, domain: &str, bearer: Option<&str>) -> Self {
        let client = match endpoint {
            Endpoint::Unix(p) => Client::unix(p.clone()),
            Endpoint::Tcp(a) => Client::tcp(a.clone()),
        };
        let client = match bearer {
            Some(t) => client.with_bearer(t),
            None => client,
        };
        Self::new(client, domain)
    }

    /// 绑定实例键（缺省 `default` 实例，D27）。
    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    /// 注入按参与者令牌（D11）。
    pub fn with_bearer(self, token: impl Into<String>) -> Self {
        Self {
            client: self.client.with_bearer(token),
            domain: self.domain,
            instance: self.instance,
        }
    }
}

#[async_trait]
impl ParticipantClient for RestClient {
    fn domain(&self) -> &str {
        &self.domain
    }

    fn instance(&self) -> Option<&str> {
        self.instance.as_deref()
    }

    async fn list_domains(&self) -> Result<Vec<DomainListing>, RuntimeError> {
        Ok(self.client.list_domains().await?.domains)
    }

    async fn load_domain(&self, declaration: &str) -> Result<(), RuntimeError> {
        self.client
            .load_domain(&self.domain, declaration, self.instance(), false)
            .await?;
        Ok(())
    }

    async fn seed(
        &self,
        participant: &str,
        ty: &str,
        thing_ref: Option<&str>,
    ) -> Result<Token, RuntimeError> {
        Ok(self
            .client
            .seed(&self.domain, self.instance(), participant, ty, thing_ref)
            .await?
            .token)
    }

    async fn produce(
        &self,
        participant: &str,
        ty: &str,
        predecessors: &[String],
    ) -> Result<Token, RuntimeError> {
        Ok(self
            .client
            .produce(
                &self.domain,
                self.instance(),
                participant,
                ty,
                predecessors.to_vec(),
                None,
            )
            .await?
            .token)
    }

    async fn read(&self, participant: &str, token_id: &str) -> Result<Token, RuntimeError> {
        Ok(self
            .client
            .read_token(&self.domain, self.instance(), participant, token_id)
            .await?
            .token)
    }

    async fn consume(
        &self,
        participant: &str,
        token_id: &str,
        request_id: Option<&str>,
    ) -> Result<ArchiveEntry, RuntimeError> {
        Ok(self
            .client
            .consume(
                &self.domain,
                self.instance(),
                participant,
                token_id,
                request_id,
            )
            .await?
            .entry)
    }

    async fn my_gates(&self, participant: &str) -> Result<Vec<GateClause>, RuntimeError> {
        Ok(self
            .client
            .gates(&self.domain, self.instance(), participant)
            .await?
            .gates)
    }

    async fn enabled(&self, participant: &str) -> Result<Vec<EnabledOption>, RuntimeError> {
        Ok(self
            .client
            .enabled(&self.domain, self.instance(), participant)
            .await?
            .options)
    }

    async fn counter(&self) -> Result<CounterView, RuntimeError> {
        Ok(self
            .client
            .counter(&self.domain, self.instance(), None)
            .await?)
    }

    async fn archive(&self) -> Result<Vec<ArchiveEntry>, RuntimeError> {
        Ok(self
            .client
            .archive(&self.domain, self.instance(), None)
            .await?
            .entries)
    }

    async fn status(&self) -> Result<StatusView, RuntimeError> {
        Ok(self.client.status(&self.domain, self.instance()).await?)
    }

    async fn subscribe(&self) -> Result<WakeStream, RuntimeError> {
        let mut stream = self.client.events(&self.domain, self.instance()).await?;
        let (tx, rx) = mpsc::unbounded_channel();
        // 泵任务：把 SSE 通知转发进唤醒通道；接收端关闭即退出。
        tokio::spawn(async move {
            while let Ok(Some(n)) = stream.next_event().await {
                if tx.send(n).is_err() {
                    break;
                }
            }
        });
        Ok(WakeStream::from_receiver(rx))
    }
}
