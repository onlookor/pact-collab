//! daemon 启动逻辑（设计 §9.2/§9.4）：`pactd` 与 `pact daemon start` 共用。
//!
//! 打开存储 → 引擎自动恢复 → unix socket / TCP 监听 → 信号关停 → 终态快照。

use crate::config::Config;
use crate::server::{router, serve_tcp, serve_unix, ServerState};
use fs4::fs_std::FileExt;
use pact_engine::{Engine, SystemClock, UlidGen};
use pact_store::Store;
use std::sync::Arc;

pub async fn run_daemon(cfg: Config) -> Result<(), Box<dyn std::error::Error>> {
    // data_dir 独占锁（D27）：单权威是**进程级**保证，两进程共用同一 data_dir
    // 会各自扫描/重建索引并交叉写入，故启动即取 flock。锁随进程存活持有。
    let _lock = acquire_data_dir_lock(&cfg.data_dir())?;

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

/// 取 `<data_dir>/pactd.lock` 的独占 flock（非阻塞）；已被占用则报错。
/// 返回的 `File` 须持有至进程退出（drop 即释放锁）。
fn acquire_data_dir_lock(data_dir: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::create_dir_all(data_dir)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data_dir.join("pactd.lock"))?;
    let acquired = file.try_lock_exclusive().map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            format!("无法获取 data_dir 独占锁（另一 pactd 实例正在运行？）：{e}"),
        )
    })?;
    if !acquired {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "无法获取 data_dir 独占锁（另一 pactd 实例正在运行？）",
        ));
    }
    Ok(file)
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

#[cfg(test)]
mod tests {
    use super::acquire_data_dir_lock;

    /// D27 护栏：同一 data_dir 的独占 flock 生效（第二个持有者失败），释放后可重取。
    #[test]
    fn data_dir_lock_is_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire_data_dir_lock(dir.path()).expect("首锁应成功");
        let err = acquire_data_dir_lock(dir.path()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);
        // 释放后（drop）可重新获取
        drop(first);
        acquire_data_dir_lock(dir.path()).expect("释放后可重获");
    }
}
