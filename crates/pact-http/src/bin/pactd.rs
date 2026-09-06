//! PACT daemon（设计 §9.2/§9.4）：解析配置路径后委托 [`pact_http::daemon`]。

use clap::Parser;
use pact_http::config::Config;
use pact_http::daemon::run_daemon;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "pactd", about = "PACT daemon（REST + SSE）")]
struct Args {
    /// 配置文件路径（文件不存在 = 全默认：./data、./pact.sock、信任声明模式）
    #[arg(long, default_value = "pact.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("pactd: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let cfg = Config::load(&args.config)?;
    run_daemon(cfg).await
}
