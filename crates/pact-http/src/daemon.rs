//! daemon 启动逻辑（设计 §9.2/§9.4）：`pactd` 与 `pact daemon start` 共用。
//!
//! 打开存储 → 引擎自动恢复 → unix socket / TCP 监听 → 信号关停 → 终态快照。

use crate::config::Config;
use crate::server::{router, serve_tcp, serve_unix, ServerState};
use pact_engine::{Engine, SystemClock, UlidGen};
use pact_store::Store;
use std::sync::Arc;

pub async fn run_daemon(cfg: Config) -> Result<(), Box<dyn std::error::Error>> {
    let store = Store::open(cfg.data_dir()).await?;
    // 引擎启动即自动恢复（actor run loop 内重放全部域日志）
    let engine = Engine::spawn(store, Arc::new(SystemClock), Arc::new(UlidGen));
    let state = ServerState::new(engine.clone(), cfg.auth_tokens());
    let app = router(state);

    let unix = cfg.listen_unix();
    let tcp = cfg.listen_tcp.clone();
    if unix.is_none() && tcp.is_none() {
        return Err("配置中无可用监听（listen_unix 被禁用且未配置 listen_tcp）".into());
    }

    println!(
        "pactd: 监听{}",
        [
            unix.as_ref()
                .map(|p| format!(" unix:{}", p.display()))
                .unwrap_or_default(),
            tcp.as_ref()
                .map(|t| format!(" tcp:{t}"))
                .unwrap_or_default(),
        ]
        .join(",")
    );

    tokio::select! {
        r = serve_listeners(app, unix.as_deref(), tcp.as_deref()) => {
            if let Err(e) = r {
                eprintln!("pactd: 监听失败：{e}");
            }
        }
        _ = tokio::signal::ctrl_c() => {
            println!("pactd: 收到中断信号，正在关停");
        }
    }

    // 关停：终态快照后退出
    engine.shutdown().await?;
    if let Some(p) = &unix {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

async fn serve_listeners(
    app: axum::Router,
    unix: Option<&std::path::Path>,
    tcp: Option<&str>,
) -> std::io::Result<()> {
    match (unix, tcp) {
        (Some(u), Some(t)) => {
            let app2 = app.clone();
            let t = t.to_owned();
            let tcp_task = tokio::spawn(async move { serve_tcp(app2, &t).await });
            tokio::select! {
                r = serve_unix(app, u) => r,
                r = tcp_task => {
                    r.map_err(std::io::Error::other)?
                }
            }
        }
        (Some(u), None) => serve_unix(app, u).await,
        (None, Some(t)) => serve_tcp(app, t).await,
        (None, None) => Ok(()),
    }
}
