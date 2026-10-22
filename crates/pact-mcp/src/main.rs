//! PACT MCP server（stdio 薄封装）：工具实现位于 [`pact_http::mcp`]，
//! 经 [`pact_http::client`] 接入 daemon；错误以 §6.3 结构化 JSON 返回。

use clap::Parser;
use pact_http::client::Client;
use pact_http::mcp::PactMcp;
use pact_http::sign::SigningIdentity;
use rmcp::ServiceExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Parser)]
#[command(name = "pact-mcp", about = "PACT MCP server（stdio）")]
struct Args {
    /// unix socket 路径（缺省 ./pact.sock）。
    #[arg(long)]
    unix: Option<PathBuf>,
    /// TCP 地址（如 127.0.0.1:8787）；提供时优先于 --unix。
    #[arg(long)]
    tcp: Option<String>,
    /// 使用 HTTPS 连接（要求同时提供 --tcp）。
    #[arg(long)]
    https: bool,
    /// 自定义 CA 证书（PEM）；仅 --https 时生效。
    #[arg(long)]
    tls_ca: Option<PathBuf>,
    /// Ed25519 签名种子文件（base64 编码的 32 字节）；须与 --sign-as 同时提供。
    #[arg(long)]
    sign_key: Option<PathBuf>,
    /// 签名参与者名；须与 --sign-key 同时提供。
    #[arg(long)]
    sign_as: Option<String>,
    /// 参与者令牌（§9.4 信任声明模式下可省略）。
    #[arg(long)]
    bearer: Option<String>,
    /// 从文件首个非空行读取参与者令牌；与 --bearer 互斥。
    #[arg(long, conflicts_with = "bearer")]
    bearer_file: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();
    let mut client = match (&args.tcp, args.https) {
        (Some(addr), true) => Client::https(addr.clone(), args.tls_ca.clone())?,
        (Some(addr), false) => Client::tcp(addr.clone()),
        (None, true) => return Err("--https 需要同时提供 --tcp".into()),
        (None, false) => Client::unix(
            args.unix
                .clone()
                .unwrap_or_else(|| PathBuf::from("./pact.sock")),
        ),
    };
    if let Some(t) = args.bearer {
        client = client.with_bearer(t);
    }
    if let Some(path) = &args.bearer_file {
        let token = read_secret_line(path).map_err(|e| format!("读取令牌文件失败：{e}"))?;
        client = client.with_bearer(token);
    }
    if args.sign_key.is_some() != args.sign_as.is_some() {
        return Err("--sign-key 与 --sign-as 必须同时提供".into());
    }
    if let (Some(path), Some(participant)) = (&args.sign_key, &args.sign_as) {
        let seed = read_secret_line(path)?;
        let id = SigningIdentity::from_seed_base64(participant, &seed)
            .map_err(|e| format!("签名身份构造失败：{e}"))?;
        client = client.with_signing(id);
    }
    let service = PactMcp::new(Arc::new(client))
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

/// 读取文件首个非空行（种子/令牌）；错误信息不回显文件内容。
fn read_secret_line(path: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("无法读取文件 {}：{e}", path.display()))?;
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("文件 {} 为空", path.display()))
}
