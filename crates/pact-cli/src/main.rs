//! pact-cli（§9.3 人类接口）：`pact` 命令。
//!
//! - 在线命令经 unix socket（默认）或 TCP 调 REST API（复用 pact-http client）；
//! - 离线命令（`analyze` / `compose` / `replay`）不经 daemon；
//! - `--json` 输出与 §6 线上格式一致。

use clap::{Parser, Subcommand};
use pact_core::CounterState;
use pact_engine::views::AnalysisReport;
use pact_http::client::{Client, ClientError};
use pact_http::config::Config;
use pact_http::daemon::run_daemon;
use pact_http::dto::{conflicts_detail, problem_detail, ErrorBody, ErrorDetail};
use pact_store::{apply_event, ApplyOutcome, Event};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "pact", version, about = "PACT 范式运行时 CLI（§9.3）")]
struct Cli {
    /// unix socket 路径（缺省 ./pact.sock）
    #[arg(long, global = true)]
    unix: Option<PathBuf>,
    /// TCP 地址（如 127.0.0.1:8787；设置时优先于 --unix）
    #[arg(long, global = true)]
    tcp: Option<String>,
    /// 参与者令牌（Authorization: Bearer；信任声明模式下可省略）
    #[arg(long, global = true)]
    bearer: Option<String>,
    /// 以 §6 线上格式输出 JSON
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// daemon 控制
    Daemon {
        #[command(subcommand)]
        cmd: DaemonCmd,
    },
    /// 域生命周期（validate / load / list / info / unload）
    Domain {
        #[command(subcommand)]
        cmd: DomainCmd,
    },
    /// Rule-S：种子注入
    Seed {
        #[arg(long)]
        domain: String,
        /// 目标实例名（缺省 default）
        #[arg(long)]
        instance: Option<String>,
        #[arg(long)]
        participant: String,
        #[arg(long = "type")]
        ty: String,
        #[arg(long)]
        thing_ref: Option<String>,
    },
    /// Rule-P：生产
    Produce {
        #[arg(long)]
        domain: String,
        /// 目标实例名（缺省 default）
        #[arg(long)]
        instance: Option<String>,
        #[arg(long)]
        participant: String,
        #[arg(long = "type")]
        ty: String,
        /// 前驱 token id（逗号分隔）
        #[arg(long, value_delimiter = ',', default_value = "")]
        predecessors: Vec<String>,
        #[arg(long)]
        thing_ref: Option<String>,
    },
    /// Rule-R：读取
    Read {
        #[arg(long)]
        domain: String,
        /// 目标实例名（缺省 default）
        #[arg(long)]
        instance: Option<String>,
        #[arg(long)]
        participant: String,
        #[arg(long)]
        token: String,
    },
    /// Rule-E：消费（归档）
    Consume {
        #[arg(long)]
        domain: String,
        /// 目标实例名（缺省 default）
        #[arg(long)]
        instance: Option<String>,
        #[arg(long)]
        participant: String,
        #[arg(long)]
        token: String,
        /// 可选幂等键：同实例内重复请求返回首次结果，不重复消费。
        #[arg(long)]
        request_id: Option<String>,
    },
    /// Counter 查询
    Counter {
        #[command(subcommand)]
        cmd: CounterCmd,
    },
    /// Archive 查询
    Archive {
        #[command(subcommand)]
        cmd: ArchiveCmd,
    },
    /// 参与者 gate 自省
    Gates {
        #[command(subcommand)]
        cmd: GatesCmd,
    },
    /// 当前可执行生产（Rule-W 自驱）
    Enabled {
        #[arg(long)]
        domain: String,
        /// 目标实例名（缺省 default）
        #[arg(long)]
        instance: Option<String>,
        #[arg(long)]
        participant: String,
    },
    /// 域状态概要
    Status {
        #[arg(long)]
        domain: String,
        /// 目标实例名（缺省 default）
        #[arg(long)]
        instance: Option<String>,
    },
    /// 离线：静态分析（不经 daemon）
    Analyze {
        /// .pact 声明文件
        file: PathBuf,
    },
    /// 离线：片段并行组合（不经 daemon）
    Compose {
        /// .pact 片段文件（≥2）
        files: Vec<PathBuf>,
    },
    /// 离线：重放事件日志
    Replay {
        /// <domain>/<instance>.events.jsonl
        file: PathBuf,
    },
}

#[derive(Subcommand)]
enum DaemonCmd {
    /// 前台启动 daemon（--config pact.toml）
    Start {
        #[arg(long, default_value = "pact.toml")]
        config: PathBuf,
    },
}

#[derive(Subcommand)]
enum DomainCmd {
    /// 校验声明文本良构（不加载）
    Validate {
        /// .pact 声明文件
        file: PathBuf,
    },
    /// 加载域（声明 + 实例化；不指定实例 ⇒ 默认实例）
    Load {
        /// .pact 声明文件
        file: PathBuf,
        /// 域名（缺省取文件 stem）
        #[arg(long)]
        name: Option<String>,
        /// 目标实例名（缺省 default）
        #[arg(long)]
        instance: Option<String>,
        /// 目标实例非空时允许替换（重置其 Counter）
        #[arg(long)]
        force: bool,
    },
    /// 在已登记声明上创建额外实例
    Instantiate {
        #[arg(long)]
        domain: String,
        #[arg(long)]
        instance: String,
    },
    /// 域清单（定义键 + 实例）
    List,
    /// 单域实例清单
    Instances {
        #[arg(long)]
        domain: String,
    },
    /// 域信息 + 状态摘要
    Info {
        #[arg(long)]
        domain: String,
        /// 目标实例名（缺省 default）
        #[arg(long)]
        instance: Option<String>,
    },
    /// 卸载实例（内存态，日志保留）
    Unload {
        #[arg(long)]
        domain: String,
        /// 目标实例名（缺省 default）
        #[arg(long)]
        instance: Option<String>,
    },
}

#[derive(Subcommand)]
enum CounterCmd {
    /// Counter 条目
    Ls {
        #[arg(long)]
        domain: String,
        /// 目标实例名（缺省 default）
        #[arg(long)]
        instance: Option<String>,
        /// 按类型过滤
        #[arg(long = "type")]
        ty: Option<String>,
    },
}

#[derive(Subcommand)]
enum ArchiveCmd {
    /// Archive 条目
    Ls {
        #[arg(long)]
        domain: String,
        /// 目标实例名（缺省 default）
        #[arg(long)]
        instance: Option<String>,
        #[arg(long = "type")]
        ty: Option<String>,
    },
}

#[derive(Subcommand)]
enum GatesCmd {
    /// 参与者可见的 gate 子句
    Show {
        #[arg(long)]
        domain: String,
        /// 目标实例名（缺省 default）
        #[arg(long)]
        instance: Option<String>,
        #[arg(long)]
        participant: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("构建 tokio 运行时失败")
        .block_on(async { run(cli).await })
}

async fn run(cli: Cli) -> ExitCode {
    let Cli {
        unix,
        tcp,
        bearer,
        json,
        command,
    } = cli;
    let conn = ConnArgs { unix, tcp, bearer };
    match command {
        // —— daemon 控制 ——
        Command::Daemon {
            cmd: DaemonCmd::Start { config },
        } => {
            let cfg = match Config::load(&config) {
                Ok(c) => c,
                Err(e) => {
                    return fail(
                        json,
                        ErrorDetail {
                            code: "Config".into(),
                            detail: serde_json::Value::Null,
                            message: e.to_string(),
                        },
                    )
                }
            };
            match run_daemon(cfg).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(
                    json,
                    ErrorDetail {
                        code: "Daemon".into(),
                        detail: serde_json::Value::Null,
                        message: e.to_string(),
                    },
                ),
            }
        }

        // —— 域生命周期 ——
        Command::Domain {
            cmd: DomainCmd::Validate { file },
        } => {
            let text = match read_file(&file, json) {
                Ok(t) => t,
                Err(c) => return c,
            };
            let c = conn.client();
            done(json, c.validate(&text).await, |r| {
                println!(
                    "ok: types={} participants={} gates={} seed_authority={}",
                    r.types.len(),
                    r.participants.len(),
                    r.gates,
                    r.seed_authority
                );
                println!("types: {}", r.types.join(", "));
                println!("participants: {}", r.participants.join(", "));
            })
        }
        Command::Domain {
            cmd:
                DomainCmd::Load {
                    file,
                    name,
                    instance,
                    force,
                },
        } => {
            let text = match read_file(&file, json) {
                Ok(t) => t,
                Err(c) => return c,
            };
            let name = name.unwrap_or_else(|| default_name(&file));
            let c = conn.client();
            done(
                json,
                c.load_domain(&name, &text, instance.as_deref(), force)
                    .await,
                |r| {
                    println!(
                        "loaded {}: types={} gates={} seed_authority={} last_seq={}",
                        r.name, r.types, r.gates, r.seed_authority, r.last_seq
                    );
                },
            )
        }
        Command::Domain {
            cmd: DomainCmd::Instantiate { domain, instance },
        } => {
            let c = conn.client();
            done(json, c.instantiate(&domain, &instance).await, |r| {
                println!(
                    "instantiated {}: types={} gates={} seed_authority={} last_seq={}",
                    r.name, r.types, r.gates, r.seed_authority, r.last_seq
                );
            })
        }
        Command::Domain {
            cmd: DomainCmd::List,
        } => {
            let c = conn.client();
            done(json, c.list_domains().await, |r| {
                if r.domains.is_empty() {
                    println!("(无域)");
                } else {
                    for d in &r.domains {
                        println!("{} [{}]", d.name, d.instances.join(", "));
                    }
                }
            })
        }
        Command::Domain {
            cmd: DomainCmd::Instances { domain },
        } => {
            let c = conn.client();
            // JSON 输出该域的实例清单（缺省域不存在时为空间量）
            let res = c.list_domains().await.map(|r| {
                r.domains
                    .into_iter()
                    .find(|d| d.name == domain)
                    .map(|d| d.instances)
                    .unwrap_or_default()
            });
            done(json, res, |instances| {
                if instances.is_empty() {
                    println!("(无实例)");
                } else {
                    for i in instances {
                        println!("{i}");
                    }
                }
            })
        }
        Command::Domain {
            cmd: DomainCmd::Info { domain, instance },
        } => {
            let c = conn.client();
            done(
                json,
                c.domain_info(&domain, instance.as_deref()).await,
                |r| {
                    println!(
                        "{}/{}: counter={} archive={} terminated={}",
                        r.name, r.instance, r.counter_size, r.archive_size, r.terminated
                    );
                },
            )
        }
        Command::Domain {
            cmd: DomainCmd::Unload { domain, instance },
        } => {
            let c = conn.client();
            done(
                json,
                c.unload_domain(&domain, instance.as_deref()).await,
                |()| {
                    println!("unloaded {domain}");
                },
            )
        }

        // —— 四类动作 ——
        Command::Seed {
            domain,
            instance,
            participant,
            ty,
            thing_ref,
        } => {
            let c = conn.client();
            done(
                json,
                c.seed(
                    &domain,
                    instance.as_deref(),
                    &participant,
                    &ty,
                    thing_ref.as_deref(),
                )
                .await,
                print_token,
            )
        }
        Command::Produce {
            domain,
            instance,
            participant,
            ty,
            predecessors,
            thing_ref,
        } => {
            let c = conn.client();
            done(
                json,
                c.produce(
                    &domain,
                    instance.as_deref(),
                    &participant,
                    &ty,
                    predecessors,
                    thing_ref.as_deref(),
                )
                .await,
                print_token,
            )
        }
        Command::Read {
            domain,
            instance,
            participant,
            token,
        } => {
            let c = conn.client();
            done(
                json,
                c.read_token(&domain, instance.as_deref(), &participant, &token)
                    .await,
                print_token,
            )
        }
        Command::Consume {
            domain,
            instance,
            participant,
            token,
            request_id,
        } => {
            let c = conn.client();
            done(
                json,
                c.consume(
                    &domain,
                    instance.as_deref(),
                    &participant,
                    &token,
                    request_id.as_deref(),
                )
                .await,
                |r| {
                    println!(
                        "consumed {} (type {}, by {} at {})",
                        r.entry.token.id,
                        r.entry.token.ty,
                        r.entry.exhausted_by,
                        r.entry.exhausted_at
                    );
                },
            )
        }

        // —— 查询 ——
        Command::Counter {
            cmd:
                CounterCmd::Ls {
                    domain,
                    instance,
                    ty,
                },
        } => {
            let c = conn.client();
            done(
                json,
                c.counter(&domain, instance.as_deref(), ty.as_deref()).await,
                |r| {
                    println!("counter: {} tokens", r.size);
                    for t in &r.tokens {
                        println!(
                            "{} type={} depends_on=[{}]",
                            t.id,
                            t.ty,
                            t.depends_on
                                .iter()
                                .map(|i| i.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }
                },
            )
        }
        Command::Archive {
            cmd:
                ArchiveCmd::Ls {
                    domain,
                    instance,
                    ty,
                },
        } => {
            let c = conn.client();
            done(
                json,
                c.archive(&domain, instance.as_deref(), ty.as_deref()).await,
                |r| {
                    println!("archive: {} entries", r.size);
                    for e in &r.entries {
                        println!(
                            "{} type={} exhausted_by={} at {}",
                            e.token.id, e.token.ty, e.exhausted_by, e.exhausted_at
                        );
                    }
                },
            )
        }
        Command::Gates {
            cmd:
                GatesCmd::Show {
                    domain,
                    instance,
                    participant,
                },
        } => {
            let c = conn.client();
            done(
                json,
                c.gates(&domain, instance.as_deref(), &participant).await,
                |r| {
                    println!("gates of {participant} in {domain}: {}", r.gates.len());
                    for g in &r.gates {
                        match &g.condition {
                            Some(expr) => println!(
                                "{} {} when {}",
                                g.action.as_str(),
                                g.ty,
                                pact_parser::emit_expr(expr)
                            ),
                            None => println!("{} {}", g.action.as_str(), g.ty),
                        }
                    }
                },
            )
        }
        Command::Enabled {
            domain,
            instance,
            participant,
        } => {
            let c = conn.client();
            done(
                json,
                c.enabled(&domain, instance.as_deref(), &participant).await,
                |r| {
                    println!("enabled productions for {participant}: {}", r.options.len());
                    for o in &r.options {
                        println!(
                            "{} witnesses=[{}]",
                            o.ty,
                            o.witness_predecessors
                                .iter()
                                .map(|i| i.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }
                },
            )
        }
        Command::Status { domain, instance } => {
            let c = conn.client();
            done(json, c.status(&domain, instance.as_deref()).await, |r| {
                println!(
                    "{}: counter={} archive={} terminated={}",
                    domain, r.counter_size, r.archive_size, r.terminated
                );
            })
        }

        // —— 离线命令 ——
        Command::Analyze { file } => offline_analyze(json, &file),
        Command::Compose { files } => offline_compose(json, &files),
        Command::Replay { file } => offline_replay(json, &file),
    }
}

// ============ 在线辅助 ============

/// 连接参数（全局 --unix / --tcp / --bearer）。
struct ConnArgs {
    unix: Option<PathBuf>,
    tcp: Option<String>,
    bearer: Option<String>,
}

impl ConnArgs {
    fn client(&self) -> Client {
        let c = match &self.tcp {
            Some(addr) => Client::tcp(addr.clone()),
            None => Client::unix(
                self.unix
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("./pact.sock")),
            ),
        };
        match &self.bearer {
            Some(t) => c.with_bearer(t.clone()),
            None => c,
        }
    }
}

/// 统一结果处理：`--json` 序列化成功值 / §6.3 错误体；否则人类可读输出。
fn done<T: serde::Serialize>(
    json: bool,
    res: Result<T, ClientError>,
    human: impl FnOnce(&T),
) -> ExitCode {
    match res {
        Ok(v) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
            } else {
                human(&v);
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(json, client_error_detail(e)),
    }
}

fn client_error_detail(e: ClientError) -> ErrorDetail {
    match e {
        ClientError::Api(d) => d,
        ClientError::Http(e) => ErrorDetail {
            code: "Transport".into(),
            detail: serde_json::Value::Null,
            message: e.to_string(),
        },
        ClientError::Io(e) => ErrorDetail {
            code: "Io".into(),
            detail: serde_json::Value::Null,
            message: e.to_string(),
        },
        ClientError::Parse(e) => ErrorDetail {
            code: "Parse".into(),
            detail: serde_json::Value::Null,
            message: e.to_string(),
        },
        ClientError::UnexpectedStatus(s) => ErrorDetail {
            code: "UnexpectedStatus".into(),
            detail: serde_json::json!({ "status": s.as_u16() }),
            message: format!("daemon 返回非预期状态码 {s}"),
        },
    }
}

// ============ 离线命令 ============

fn offline_analyze(json: bool, file: &PathBuf) -> ExitCode {
    let text = match read_file(file, json) {
        Ok(t) => t,
        Err(c) => return c,
    };
    let domain = match pact_parser::load_domain(&text) {
        Ok(d) => d,
        Err(p) => return fail(json, problem_detail(&p)),
    };
    let report = AnalysisReport {
        precondition_wf: pact_analysis::precondition_wf(&domain),
        reachability: pact_analysis::reachability(&domain),
        gate_coverage: pact_analysis::gate_coverage(&domain),
        consumption_reachability: pact_analysis::consumption_reachability(&domain),
        structural_deadlock: pact_analysis::structural_deadlock(&domain),
        stranding_risk: pact_analysis::stranding_risk(&domain),
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        // A.5 先输出（Appendix D：良构性是其余分析的前提）
        println!(
            "precondition_wf: {}（{} 项违规，{} 项警告）",
            if report.precondition_wf.well_formed {
                "良构"
            } else {
                "非良构"
            },
            report.precondition_wf.violations.len(),
            report.precondition_wf.warnings.len()
        );
        for w in &report.precondition_wf.warnings {
            println!("  warning: {w}");
        }
        println!(
            "reachability: {} 源类型，{} 可达，{} 不可达",
            report.reachability.seed_types.len(),
            report.reachability.reachable.len(),
            report.reachability.unreachable.len()
        );
        if !report.reachability.unreachable.is_empty() {
            println!(
                "  unreachable: {}",
                join_types(&report.reachability.unreachable)
            );
        }
        println!(
            "gate_coverage: {} 个无 producer 的非源类型",
            report.gate_coverage.len()
        );
        println!(
            "consumption_reachability: {} 个无 consumer 的类型",
            report.consumption_reachability.len()
        );
        println!(
            "structural_deadlock: {} 个强连通分量",
            report.structural_deadlock.len()
        );
        println!("stranding_risk: {} 处滞留风险", report.stranding_risk.len());
    }
    ExitCode::SUCCESS
}

fn offline_compose(json: bool, files: &[PathBuf]) -> ExitCode {
    if files.len() < 2 {
        return fail(
            json,
            ErrorDetail {
                code: "Usage".into(),
                detail: serde_json::Value::Null,
                message: "compose 至少需要两个片段文件".into(),
            },
        );
    }
    let mut fragments = Vec::with_capacity(files.len());
    for f in files {
        let text = match read_file(f, json) {
            Ok(t) => t,
            Err(c) => return c,
        };
        match pact_parser::load_domain(&text) {
            Ok(d) => fragments.push(d),
            Err(p) => return fail(json, problem_detail(&p)),
        }
    }
    match pact_compose::compose(&fragments) {
        Ok(merged) => {
            if json {
                let participants: std::collections::BTreeSet<_> =
                    merged.gates.keys().map(|(p, _, _)| p.clone()).collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "types": merged.types.len(),
                        "participants": participants.len(),
                        "gates": merged.gates.len(),
                        "seed_authority": merged.seed_authority.len(),
                    }))
                    .unwrap()
                );
            } else {
                println!("{}", pact_parser::emit_domain(&merged));
            }
            ExitCode::SUCCESS
        }
        Err(conflicts) => fail(json, conflicts_detail(&conflicts)),
    }
}

fn offline_replay(json: bool, file: &PathBuf) -> ExitCode {
    let text = match read_file(file, json) {
        Ok(t) => t,
        Err(c) => return c,
    };
    let mut state = CounterState::new();
    let mut domain_name: Option<String> = None;
    let mut applied = 0u64;
    for (idx, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let ev: Event = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(e) => {
                return fail(
                    json,
                    ErrorDetail {
                        code: "Parse".into(),
                        detail: serde_json::json!({ "line": idx + 1 }),
                        message: format!("第 {} 行不是合法事件：{e}", idx + 1),
                    },
                )
            }
        };
        match apply_event(&mut state, &ev) {
            Ok(ApplyOutcome::Applied) => applied += 1,
            Ok(ApplyOutcome::DomainReset { name, .. }) => {
                state = CounterState::new();
                domain_name = Some(name);
            }
            Ok(ApplyOutcome::Informational) => {}
            Err(e) => {
                return fail(
                    json,
                    ErrorDetail {
                        code: "Replay".into(),
                        detail: serde_json::json!({ "line": idx + 1 }),
                        message: format!("第 {} 行应用失败：{e}", idx + 1),
                    },
                )
            }
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&state).unwrap());
    } else {
        println!(
            "replayed {} events (domain {}): counter={} archive={}",
            applied,
            domain_name.as_deref().unwrap_or("-"),
            state.counter.len(),
            state.archive.len()
        );
    }
    ExitCode::SUCCESS
}

// ============ 公共辅助 ============

fn read_file(path: &PathBuf, json: bool) -> Result<String, ExitCode> {
    std::fs::read_to_string(path).map_err(|e| {
        fail(
            json,
            ErrorDetail {
                code: "Io".into(),
                detail: serde_json::json!({ "path": path.display().to_string() }),
                message: format!("读取 {} 失败：{e}", path.display()),
            },
        )
    })
}

/// 域名缺省取文件 stem（如 `dev.pact` → `dev`）。
fn default_name(file: &Path) -> String {
    file.file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| "domain".to_owned())
}

fn join_types(ts: &[pact_core::TypeName]) -> String {
    ts.iter()
        .map(|t| t.0.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

fn print_token(r: &pact_http::dto::TokenResp) {
    println!(
        "token {} type={} produced_by={} depends_on=[{}]",
        r.token.id,
        r.token.ty,
        r.token.produced_by,
        r.token
            .depends_on
            .iter()
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// 错误统一出口：`--json` → §6.3 错误体到 stdout；人类模式 → 简明信息到 stderr。
fn fail(json: bool, detail: ErrorDetail) -> ExitCode {
    if json {
        println!(
            "{}",
            serde_json::to_string(&ErrorBody {
                error: detail.clone()
            })
            .unwrap()
        );
    } else {
        eprintln!("error [{}]: {}", detail.code, detail.message);
    }
    ExitCode::FAILURE
}
