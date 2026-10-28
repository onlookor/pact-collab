//! `pactd` 进程级生命周期 e2e（D25 子项 2）：PID 文件与 SIGTERM 优雅停机。
//!
//! 以子进程拉起真实 `pactd` 二进制，验证：启动写 PID 文件 → SIGTERM → 优雅退出（码 0）
//! → PID 文件与套接字被清理。

use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

/// 轮询等待条件成立（上限 ~10s）。
fn wait_for<F: Fn() -> bool>(cond: F, what: &str) {
    for _ in 0..400 {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("等待超时：{what}");
}

/// 轮询等待子进程退出（上限 ~10s）。
fn wait_child(child: &mut Child) -> ExitStatus {
    for _ in 0..400 {
        if let Some(st) = child.try_wait().unwrap() {
            return st;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    panic!("进程未在期限内退出");
}

fn write_config(dir: &Path, data: &Path, sock: &Path, pid: &Path) -> std::path::PathBuf {
    let cfg = dir.join("pact.toml");
    std::fs::write(
        &cfg,
        format!(
            "data_dir = \"{}\"\nlisten_unix = \"{}\"\npid_file = \"{}\"\n",
            data.display(),
            sock.display(),
            pid.display()
        ),
    )
    .unwrap();
    cfg
}

/// D42：RefBindings 非法 → 拒绝启动（fail-closed），不创建 PID/套接字。
#[test]
fn invalid_ref_bindings_abort_startup() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let sock = dir.path().join("pact.sock");
    let pid = dir.path().join("pactd.pid");
    let bad = dir.path().join("bad.refs.yaml");
    std::fs::write(
        &bad,
        "apiVersion: pact.ref/v1\nkind: RefBindings\nmetadata:\n  domain: proj\nspec:\n  types:\n    design:\n      ref:\n        contract: \"not a contract id\"\n",
    )
    .unwrap();
    let cfg = dir.path().join("pact.toml");
    std::fs::write(
        &cfg,
        format!(
            "data_dir = \"{}\"\nlisten_unix = \"{}\"\npid_file = \"{}\"\n[ref_bindings]\npaths = [\"{}\"]\n",
            data.display(),
            sock.display(),
            pid.display(),
            bad.display()
        ),
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_pactd"))
        .arg("--config")
        .arg(&cfg)
        .output()
        .expect("应能拉起 pactd");
    assert!(!out.status.success(), "非法 RefBindings 应拒绝启动");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("RefBindings"), "stderr: {stderr}");
    assert!(!pid.exists(), "拒绝启动时不应写 PID 文件");
    assert!(!sock.exists(), "拒绝启动时不应创建套接字");
}

/// SIGTERM 触发优雅停机：进程以 0 退出，PID 文件与套接字被清理。
#[test]
fn sigterm_triggers_graceful_shutdown_and_clears_pid() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let sock = dir.path().join("pact.sock");
    let pid_path = dir.path().join("pactd.pid");
    let cfg = write_config(dir.path(), &data, &sock, &pid_path);

    let mut child = Command::new(env!("CARGO_BIN_EXE_pactd"))
        .arg("--config")
        .arg(&cfg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("应能拉起 pactd");

    // 就绪：PID 文件与套接字均已出现
    wait_for(
        || pid_path.exists() && sock.exists(),
        "pactd 就绪（PID 文件 + 套接字）",
    );
    let pid = std::fs::read_to_string(&pid_path)
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    assert_eq!(pid, child.id(), "PID 文件应记录本进程 PID");

    // 发 SIGTERM
    let killed = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .expect("应能执行 kill")
        .success();
    assert!(killed, "kill -TERM 应成功");

    // 优雅退出：码 0
    let status = wait_child(&mut child);
    assert!(status.success(), "SIGTERM 后应以 0 退出：{status:?}");

    // 清理：PID 文件与套接字均删除
    assert!(!pid_path.exists(), "PID 文件应被删除");
    assert!(!sock.exists(), "套接字应被删除");
}

/// 陈旧 PID 文件指向存活进程 → 拒绝启动（退出码 1）。
#[test]
fn stale_pid_file_with_live_process_refuses_start() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let sock = dir.path().join("pact.sock");
    let pid_path = dir.path().join("pactd.pid");
    let cfg = write_config(dir.path(), &data, &sock, &pid_path);

    // 写入「当前测试进程」的 PID（必然存活）模拟陈旧文件
    std::fs::write(&pid_path, format!("{}\n", std::process::id())).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_pactd"))
        .arg("--config")
        .arg(&cfg)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("应能执行 pactd");
    assert!(!out.status.success(), "应拒绝启动");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("仍在运行的进程"), "stderr={err}");
    // 未监听：套接字不应被创建
    assert!(!sock.exists(), "拒绝启动时不应创建套接字");
}
