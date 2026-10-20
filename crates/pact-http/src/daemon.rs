//! daemon 启动逻辑（设计 §9.2/§9.4）：`pactd` 与 `pact daemon start` 共用。
//!
//! 打开存储 → 引擎自动恢复 → unix socket / TCP 监听 → 信号关停 → 终态快照。
//! D25 子项 2：可选 PID 文件（含陈旧检测）、SIGTERM/SIGINT 优雅停机（有界 drain）。

use crate::config::Config;
use crate::server::{router, serve_tcp, serve_unix, ConnTracker, ServerState};
use fs4::fs_std::FileExt;
use pact_engine::{Engine, EngineConfig, SystemClock, UlidGen};
use pact_store::Store;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// 停机时等待在途连接完成的上限（超过即强制退出，仍会做终态快照）。
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn run_daemon(cfg: Config) -> Result<(), Box<dyn std::error::Error>> {
    // data_dir 独占锁（D27）：单权威是**进程级**保证，两进程共用同一 data_dir
    // 会各自扫描/重建索引并交叉写入，故启动即取 flock。锁随进程存活持有。
    let _lock = acquire_data_dir_lock(&cfg.data_dir())?;
    // PID 文件（D25）：可选；陈旧文件且 PID 仍存活 → 拒绝启动。退出时自动删除。
    let _pid = PidFile::acquire(cfg.pid_file.as_deref())?;

    let store = Store::open(cfg.data_dir()).await?;
    // 引擎启动即自动恢复（actor run loop 内重放全部域日志）；
    // 压缩配置（D25 子项 1）随 `pact.toml` 传入。
    let engine_config = EngineConfig {
        compact_on_start: cfg.compact_on_start(),
        compact_keep_last: cfg.compact_keep_last(),
        ..EngineConfig::default()
    };
    let engine = Engine::spawn_with_config(
        store,
        Arc::new(SystemClock),
        Arc::new(UlidGen),
        engine_config,
    );
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

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let tracker = ConnTracker::default();
    let listeners = serve_listeners(
        app,
        unix.as_deref(),
        tcp.as_deref(),
        shutdown_rx,
        tracker.clone(),
    );
    tokio::pin!(listeners);

    let listeners_done = tokio::select! {
        r = &mut listeners => {
            if let Err(e) = r {
                eprintln!("pactd: 监听失败：{e}");
            }
            true
        }
        _ = shutdown_signal() => {
            println!("pactd: 收到停机信号，停止接受新连接");
            false
        }
    };

    // 停机序列：广播停机 → 监听循环退出（停止 accept）→ 有界等待在途连接 → 终态快照
    let _ = shutdown_tx.send(true);
    if !listeners_done {
        let _ = tokio::time::timeout(DRAIN_TIMEOUT, &mut listeners).await;
    }
    if tokio::time::timeout(DRAIN_TIMEOUT, tracker.wait_idle())
        .await
        .is_err()
    {
        eprintln!(
            "pactd: {} 秒内仍有 {} 条连接未结束，强制进入终态快照",
            DRAIN_TIMEOUT.as_secs(),
            tracker.active()
        );
    }
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

/// 可选 PID 文件：启动写入、退出删除；陈旧文件仅在 PID 仍存活时拒绝启动。
#[derive(Debug)]
struct PidFile {
    path: PathBuf,
}

impl PidFile {
    fn acquire(path: Option<&Path>) -> std::io::Result<Option<Self>> {
        let Some(path) = path else {
            return Ok(None);
        };
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        // 陈旧检测：文件残留且记录的进程仍在 → 拒绝启动（防止两个 pactd 争同一 pid 文件）。
        if let Some(pid) = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
        {
            if pid_alive(pid) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!(
                        "PID 文件 {} 指向仍在运行的进程 {pid}（陈旧文件请先确认并删除）",
                        path.display()
                    ),
                ));
            }
        }
        std::fs::write(path, format!("{}\n", std::process::id()))?;
        Ok(Some(Self {
            path: path.to_owned(),
        }))
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// 进程存活判定。Linux 下以 `/proc/<pid>` 存在为准；其他平台不判定（视为不存活，
/// 由 data_dir flock 兜底单实例约束）。
fn pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
}

/// 停机信号：SIGINT 或 SIGTERM（均走同一优雅停机路径）。
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("pactd: 注册 SIGTERM 失败（仅响应 SIGINT）：{e}");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => println!("pactd: 收到 SIGINT"),
            _ = term.recv() => println!("pactd: 收到 SIGTERM"),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        println!("pactd: 收到中断信号");
    }
}

async fn serve_listeners(
    app: axum::Router,
    unix: Option<&std::path::Path>,
    tcp: Option<&str>,
    shutdown: watch::Receiver<bool>,
    tracker: ConnTracker,
) -> std::io::Result<()> {
    match (unix, tcp) {
        (Some(u), Some(t)) => {
            let app2 = app.clone();
            let t = t.to_owned();
            let (tcp_shutdown, tcp_tracker) = (shutdown.clone(), tracker.clone());
            let tcp_task =
                tokio::spawn(async move { serve_tcp(app2, &t, tcp_shutdown, tcp_tracker).await });
            tokio::select! {
                r = serve_unix(app, u, shutdown, tracker) => r,
                r = tcp_task => {
                    r.map_err(std::io::Error::other)?
                }
            }
        }
        (Some(u), None) => serve_unix(app, u, shutdown, tracker).await,
        (None, Some(t)) => serve_tcp(app, t, shutdown, tracker).await,
        (None, None) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{acquire_data_dir_lock, PidFile};

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

    /// D25 护栏：PID 文件写入自身 PID；drop 后删除。
    #[test]
    fn pid_file_written_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pactd.pid");
        {
            let _pid = PidFile::acquire(Some(&path)).unwrap().expect("应写入");
            let text = std::fs::read_to_string(&path).unwrap();
            assert_eq!(text.trim(), std::process::id().to_string());
        }
        assert!(!path.exists(), "drop 后应删除 PID 文件");
    }

    /// D25 护栏：陈旧 PID 文件指向仍存活的进程 → 拒绝启动；不存在的 PID → 覆盖。
    #[test]
    fn pid_file_stale_detection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pactd.pid");

        // 指向自身（必然存活）→ 拒绝
        std::fs::write(&path, format!("{}\n", std::process::id())).unwrap();
        let err = PidFile::acquire(Some(&path)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);

        // 指向几乎不可能存活的 PID → 允许覆盖
        std::fs::write(&path, "4294967294\n").unwrap();
        let pid = PidFile::acquire(Some(&path)).unwrap();
        assert!(pid.is_some());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            std::process::id().to_string()
        );
    }

    /// 未配置 pid_file 时不写任何文件。
    #[test]
    fn pid_file_absent_when_unconfigured() {
        assert!(PidFile::acquire(None).unwrap().is_none());
    }
}
