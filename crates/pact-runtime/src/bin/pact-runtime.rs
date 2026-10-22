//! `pact-runtime`：TOML 配置驱动的生产编排入口（D24）。
//!
//! 假定 daemon 已由外部机制启动（systemd / `pact daemon start` / `pactd`）；本 bin 负责：
//! 读取 `.pact` 域声明 → 并发拉起 N 个 MCP/REST 参与者自驱至终止 → （可选）运行外部副作用
//! 执行器 → 输出各参与者报告 JSON。
//!
//! 用法：`pact-runtime --config runtime.toml`

use clap::Parser;
use pact_core::ArchiveEntry;
use pact_http::sign::SigningIdentity;
use pact_runtime::{
    AgentSpec, Bootstrap, Compensation, Endpoint, ExecutorConfig, ExecutorReport, Orchestrator,
    RestClient, SideEffectExecutor, SideEffectHandler, StopSignal, DEFAULT_EXECUTOR_POLL,
};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

/// 运行时配置根。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    /// 绑定域名。
    domain: String,
    /// 绑定实例名（缺省 `default` 实例，D27）。
    #[serde(default)]
    instance: Option<String>,
    /// `.pact` 域声明文件路径。
    declaration: PathBuf,
    /// daemon 端点。
    endpoint: EndpointCfg,
    /// 可选按参与者令牌（D11）。
    #[serde(default)]
    bearer: Option<String>,
    /// 轮询兜底间隔（毫秒）。
    #[serde(default)]
    poll_ms: Option<u64>,
    /// 自驱期限（秒）。
    #[serde(default)]
    deadline_s: Option<u64>,
    /// 参与者清单。
    #[serde(default)]
    agents: Vec<AgentCfg>,
    /// 可选外部副作用执行器。
    #[serde(default)]
    executor: Option<ExecutorCfg>,
}

/// daemon 端点（二者必居其一）。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EndpointCfg {
    #[serde(default)]
    unix: Option<PathBuf>,
    #[serde(default)]
    tcp: Option<String>,
    /// 经 HTTPS 连接 TCP 端点（D34）。
    #[serde(default)]
    https: bool,
    /// 自定义 CA PEM（仅 `https = true` 时允许）。
    #[serde(default)]
    tls_ca: Option<PathBuf>,
}

impl EndpointCfg {
    fn resolve(self) -> Result<Endpoint, String> {
        match (self.unix, self.tcp) {
            (Some(p), None) => {
                if self.https || self.tls_ca.is_some() {
                    return Err("endpoint.https / endpoint.tls_ca 仅适用于 tcp 端点".to_owned());
                }
                Ok(Endpoint::unix(p))
            }
            (None, Some(a)) => {
                if self.https {
                    Ok(Endpoint::https(a, self.tls_ca))
                } else if self.tls_ca.is_some() {
                    Err("endpoint.tls_ca 需要同时设置 endpoint.https = true".to_owned())
                } else {
                    Ok(Endpoint::tcp(a))
                }
            }
            _ => Err("endpoint 必须且只能提供 unix 或 tcp 之一".to_owned()),
        }
    }
}

/// 单个参与者配置。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentCfg {
    /// 参与者名（域内身份）。
    participant: String,
    /// 接入方式：`rest`（缺省）或 `mcp`。
    #[serde(default = "default_transport")]
    transport: String,
    /// `transport = "mcp"` 时的可执行文件路径（缺省 `pact-mcp`）。
    #[serde(default)]
    mcp_bin: Option<PathBuf>,
    /// 引导配置：作为域加载者并注入初始种子。
    #[serde(default)]
    seed: Option<SeedCfg>,
    /// Rule-P 产出预算。
    #[serde(default)]
    produce_budget: Option<usize>,
}

fn default_transport() -> String {
    "rest".to_owned()
}

/// 引导种子配置。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeedCfg {
    /// 种子类型（源类型）。
    #[serde(rename = "type")]
    ty: String,
    /// 可选外部事物引用。
    #[serde(default)]
    thing_ref: Option<String>,
}

/// 执行器配置。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutorCfg {
    /// 执行器参与者名。
    participant: String,
    /// 触发外部动作的消费类型（缺省：全部）。
    #[serde(default)]
    effect_types: Vec<String>,
    /// 失败补偿。
    #[serde(default)]
    compensation: Option<CompensationCfg>,
    /// 补偿事实追加后是否立刻归档（要求持有其 consume 门）。
    #[serde(default)]
    finalize: bool,
}

/// 补偿配置。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompensationCfg {
    /// 追加补偿事实的参与者。
    participant: String,
    /// 补偿事实类型（源类型）。
    ty: String,
}

/// 占位外部副作用处理器：打印消费事实并视为成功。
///
/// 真实外部动作由**域适配器**（L2）提供；本 bin 仅作编排演示与接线位。
struct LoggingHandler;

#[async_trait::async_trait]
impl SideEffectHandler for LoggingHandler {
    async fn apply(&self, entry: &ArchiveEntry) -> Result<(), String> {
        eprintln!(
            "[pact-runtime] 外部动作（占位处理器）：type={} token={} by={}",
            entry.token.ty.0, entry.token.id.0, entry.exhausted_by.0
        );
        Ok(())
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "pact-runtime",
    about = "PACT 生产运行层：多 agent 编排 + 外部副作用执行器（D24）"
)]
struct Args {
    /// 运行时 TOML 配置路径。
    #[arg(long)]
    config: PathBuf,
    /// 以 HTTPS 连接 TCP 端点（覆盖 `endpoint.https`）。
    #[arg(long)]
    https: bool,
    /// 自定义 CA PEM（仅 `--https` 时可用）。
    #[arg(long = "tls-ca")]
    tls_ca: Option<PathBuf>,
    /// Ed25519 签名种子文件（内容为 base64(32 字节)）。
    #[arg(long = "sign-key")]
    sign_key: Option<PathBuf>,
    /// 签名参与者名（必须与 `--sign-key` 同时提供）。
    #[arg(long = "sign-as")]
    sign_as: Option<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args.config, &args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[pact-runtime] 失败：{e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(config_path: &Path, args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(config_path)
        .map_err(|e| format!("读取配置 {config_path:?} 失败：{e}"))?;
    let cfg: FileConfig = toml::from_str(&text).map_err(|e| format!("解析配置失败：{e}"))?;
    let declaration = std::fs::read_to_string(&cfg.declaration)
        .map_err(|e| format!("读取域声明 {:?} 失败：{e}", cfg.declaration))?;
    let endpoint = apply_cli_tls(cfg.endpoint.resolve()?, args)?;
    let bearer = cfg.bearer.clone();
    let signing = load_signing(args)?;

    let mut orch = Orchestrator::new(cfg.domain.clone());
    if let Some(i) = &cfg.instance {
        orch = orch.with_instance(i.clone());
    }
    if let Some(ms) = cfg.poll_ms {
        orch = orch.with_poll(Duration::from_millis(ms));
    }
    if let Some(s) = cfg.deadline_s {
        orch = orch.with_deadline(Duration::from_secs(s));
    }
    for a in &cfg.agents {
        let mut spec = match a.transport.as_str() {
            "rest" => AgentSpec::rest(a.participant.clone(), endpoint.clone()),
            "mcp" => AgentSpec::mcp(
                a.participant.clone(),
                a.mcp_bin
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("pact-mcp")),
                endpoint.clone(),
            ),
            other => {
                return Err(format!("参与者 {} 的 transport 非法：{other}", a.participant).into())
            }
        };
        if let Some(t) = &bearer {
            spec = spec.with_bearer(t.clone());
        }
        if let Some(id) = &signing {
            spec = spec.with_signing(id.clone());
        }
        if let Some(b) = &a.seed {
            spec = spec.with_bootstrap(Bootstrap {
                declaration: declaration.clone(),
                seed_type: b.ty.clone(),
                thing_ref: b.thing_ref.clone(),
            });
        }
        if let Some(n) = a.produce_budget {
            spec = spec.with_produce_budget(n);
        }
        orch = orch.with_agent(spec);
    }

    let agents_fut = orch.run();

    let Some(ex) = &cfg.executor else {
        let agents = agents_fut.await?;
        print_report(&serde_json::json!({ "agents": agents }))?;
        return Ok(());
    };

    let stop = StopSignal::new();
    let exec_cfg = ExecutorConfig {
        participant: ex.participant.clone(),
        effect_types: ex.effect_types.clone(),
        compensation: ex.compensation.as_ref().map(|c| Compensation {
            participant: c.participant.clone(),
            ty: c.ty.clone(),
            finalize: ex.finalize,
        }),
        poll: DEFAULT_EXECUTOR_POLL,
    };
    let mut client = RestClient::from_endpoint(&endpoint, &cfg.domain, bearer.as_deref())?;
    if let Some(id) = &signing {
        client = client.with_signing(id.clone());
    }
    if let Some(i) = &cfg.instance {
        client = client.with_instance(i.clone());
    }
    let mut executor = SideEffectExecutor::new(client, exec_cfg);
    let stop_for_exec = stop.clone();
    let exec_fut = async move {
        let r = executor.run(&LoggingHandler, &stop_for_exec).await;
        stop.stop(); // 编排先结束则让执行器随之停止
        r
    };

    let (agents_res, exec_res) = tokio::join!(agents_fut, exec_fut);
    let agents = agents_res.map_err(|e| e.to_string())?;
    let executor: ExecutorReport = exec_res.map_err(|e| e.to_string())?;
    print_report(&serde_json::json!({ "agents": agents, "executor": executor }))?;
    Ok(())
}

/// 应用命令行 TLS 覆盖（`--https` / `--tls-ca`）。
fn apply_cli_tls(endpoint: Endpoint, args: &Args) -> Result<Endpoint, String> {
    if !args.https && args.tls_ca.is_none() {
        return Ok(endpoint);
    }
    match endpoint {
        Endpoint::Tcp {
            addr,
            https,
            tls_ca,
        } => {
            let https = https || args.https;
            let tls_ca = args.tls_ca.clone().or(tls_ca);
            if tls_ca.is_some() && !https {
                return Err("--tls-ca 仅在 --https 时可用".to_owned());
            }
            Ok(Endpoint::Tcp {
                addr,
                https,
                tls_ca,
            })
        }
        Endpoint::Unix(_) => Err("--https / --tls-ca 仅适用于 tcp 端点".to_owned()),
    }
}

/// 读取全局 `--sign-key/--sign-as` 并构造签名身份（两者必须成对）。
fn load_signing(args: &Args) -> Result<Option<SigningIdentity>, Box<dyn std::error::Error>> {
    match (&args.sign_key, &args.sign_as) {
        (Some(path), Some(name)) => {
            let seed = std::fs::read_to_string(path)
                .map_err(|e| format!("读取 --sign-key {} 失败：{e}", path.display()))?;
            Ok(Some(SigningIdentity::from_seed_base64(name, seed.trim())?))
        }
        (None, None) => Ok(None),
        _ => Err("--sign-key 与 --sign-as 必须同时提供".into()),
    }
}

fn print_report(value: &serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
