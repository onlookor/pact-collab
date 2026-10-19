//! 多 agent 编排：并发拉起 N 个真实 MCP（stdio 子进程）/ REST 参与者并各自自驱至终止。

use crate::client::Endpoint;
use crate::driver::{drive, AgentReport, Bootstrap, DriverConfig, DEFAULT_DEADLINE, DEFAULT_POLL};
use crate::error::RuntimeError;
use crate::mcp::McpClient;
use crate::rest::RestClient;
use std::path::PathBuf;
use std::time::Duration;

/// 参与者接入方式。
#[derive(Debug, Clone)]
pub enum Transport {
    /// REST（进程内客户端，经 unix socket / TCP 接入 daemon）。
    Rest {
        /// daemon 端点。
        endpoint: Endpoint,
        /// 可选按参与者令牌（D11）。
        bearer: Option<String>,
    },
    /// MCP stdio：拉起 `bin` 子进程并完成握手。
    Mcp {
        /// `pact-mcp` 可执行文件路径。
        bin: PathBuf,
        /// daemon 端点（作为 `--unix/--tcp` 传给子进程）。
        endpoint: Endpoint,
        /// 可选按参与者令牌。
        bearer: Option<String>,
    },
}

/// 单个参与者规格。
#[derive(Debug, Clone)]
pub struct AgentSpec {
    /// 参与者名（域内身份）。
    pub participant: String,
    /// 接入方式。
    pub transport: Transport,
    /// 引导配置（加载域 + 播种）；`None` = 等待域出现。
    pub bootstrap: Option<Bootstrap>,
    /// Rule-P 产出预算。
    pub produce_budget: Option<usize>,
}

impl AgentSpec {
    /// REST 接入的参与者。
    pub fn rest(participant: impl Into<String>, endpoint: Endpoint) -> Self {
        Self {
            participant: participant.into(),
            transport: Transport::Rest {
                endpoint,
                bearer: None,
            },
            bootstrap: None,
            produce_budget: None,
        }
    }

    /// MCP（stdio 子进程）接入的参与者。
    pub fn mcp(
        participant: impl Into<String>,
        bin: impl Into<PathBuf>,
        endpoint: Endpoint,
    ) -> Self {
        Self {
            participant: participant.into(),
            transport: Transport::Mcp {
                bin: bin.into(),
                endpoint,
                bearer: None,
            },
            bootstrap: None,
            produce_budget: None,
        }
    }

    /// 设引导配置（域加载 + 播种）。
    pub fn with_bootstrap(mut self, bootstrap: Bootstrap) -> Self {
        self.bootstrap = Some(bootstrap);
        self
    }

    /// 设 Rule-P 产出预算。
    pub fn with_produce_budget(mut self, n: usize) -> Self {
        self.produce_budget = Some(n);
        self
    }

    /// 设按参与者令牌。
    pub fn with_bearer(mut self, token: impl Into<String>) -> Self {
        match &mut self.transport {
            Transport::Rest { bearer, .. } | Transport::Mcp { bearer, .. } => {
                *bearer = Some(token.into())
            }
        }
        self
    }
}

/// 多 agent 编排器。
pub struct Orchestrator {
    domain: String,
    poll: Duration,
    deadline: Duration,
    agents: Vec<AgentSpec>,
}

impl Orchestrator {
    /// 以绑定域构造（默认轮询/期限）。
    pub fn new(domain: impl Into<String>) -> Self {
        Self {
            domain: domain.into(),
            poll: DEFAULT_POLL,
            deadline: DEFAULT_DEADLINE,
            agents: Vec::new(),
        }
    }

    /// 追加一个参与者。
    pub fn with_agent(mut self, spec: AgentSpec) -> Self {
        self.agents.push(spec);
        self
    }

    /// 设轮询兜底间隔。
    pub fn with_poll(mut self, poll: Duration) -> Self {
        self.poll = poll;
        self
    }

    /// 设自驱期限。
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// 并发自驱全部参与者；全部成功则返回各自报告，任一失败聚合为 [`RuntimeError`]。
    pub async fn run(self) -> Result<Vec<AgentReport>, RuntimeError> {
        let Orchestrator {
            domain,
            poll,
            deadline,
            agents,
        } = self;
        let mut names = Vec::with_capacity(agents.len());
        let mut handles = Vec::with_capacity(agents.len());

        for spec in agents {
            names.push(spec.participant.clone());
            let domain = domain.clone();
            handles.push(tokio::spawn(async move {
                let cfg = DriverConfig {
                    participant: spec.participant,
                    bootstrap: spec.bootstrap,
                    produce_budget: spec.produce_budget,
                    poll,
                    deadline,
                };
                match spec.transport {
                    Transport::Rest { endpoint, bearer } => {
                        let client =
                            RestClient::from_endpoint(&endpoint, &domain, bearer.as_deref());
                        drive(client, &cfg).await
                    }
                    Transport::Mcp {
                        bin,
                        endpoint,
                        bearer,
                    } => {
                        let client =
                            McpClient::spawn(&bin, &endpoint, bearer.as_deref(), domain).await?;
                        drive(client, &cfg).await
                    }
                }
            }));
        }

        let mut reports = Vec::with_capacity(handles.len());
        let mut failures = Vec::new();
        for (name, handle) in names.iter().zip(handles) {
            match handle.await {
                Ok(Ok(rep)) => reports.push(rep),
                Ok(Err(e)) => failures.push(format!("{name}: {e}")),
                Err(e) => failures.push(format!("{name}: 任务异常终止（{e}）")),
            }
        }

        if !failures.is_empty() {
            return Err(RuntimeError::Protocol(format!(
                "{} 个 agent 失败（已完成 {} 个）：\n  {}",
                failures.len(),
                reports.len(),
                failures.join("\n  ")
            )));
        }
        Ok(reports)
    }
}
