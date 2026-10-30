//! pact-cli 端到端：真实 daemon 子进程 + `pact` 命令全生命周期（§9.3）。
//!
//! - 在线：`daemon start --config` 起子进程 → load/seed/produce/read/consume/查询 → 错误路径；
//! - 离线：analyze / compose / replay（不经 daemon），并用 daemon 产出的事件日志做回放对照。

use serde_json::Value;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const DECL: &str = r#"
types { requirement; design; implementation }

preconditions {
    requirement → design;
    design → implementation
}

gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: consumes design, produces implementation;
    user: consumes implementation
}

seed_authority { external: seeds requirement }
"#;

/// 起一个真实 `pact daemon start` 子进程（临时配置），返回子进程句柄与套接字路径。
fn start_daemon(dir: &Path) -> (Child, std::path::PathBuf) {
    start_daemon_with(dir, "")
}

/// 同 `start_daemon`，附加配置片段（如 `[ref_bindings]`）。
fn start_daemon_with(dir: &Path, extra: &str) -> (Child, std::path::PathBuf) {
    let bin = env!("CARGO_BIN_EXE_pact");
    let sock = dir.join("pact.sock");
    let data = dir.join("data");
    let config = dir.join("pact.toml");
    std::fs::write(
        &config,
        format!(
            "data_dir = \"{}\"\nlisten_unix = \"{}\"\n{extra}",
            data.display(),
            sock.display()
        ),
    )
    .unwrap();
    let mut child = Command::new(bin)
        .args(["daemon", "start", "--config"])
        .arg(&config)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("启动 daemon 子进程失败");
    // 轮询套接字就绪
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut ready = false;
    while Instant::now() <= deadline {
        if std::os::unix::net::UnixStream::connect(&sock).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if !ready {
        // 防僵尸：失败路径必须回收子进程
        let _ = child.kill();
        let _ = child.wait();
        panic!("daemon 套接字未就绪");
    }
    (child, sock)
}

/// 执行一条 `pact` 命令。
fn pact(sock: &Path, json: bool, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pact"));
    cmd.arg("--unix").arg(sock);
    if json {
        cmd.arg("--json");
    }
    cmd.args(args).output().expect("执行 pact 命令失败")
}

fn ok_json(out: &Output) -> Value {
    assert!(
        out.status.success(),
        "命令应成功，stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("输出应为合法 JSON")
}

#[test]
fn lifecycle_through_real_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let (mut daemon, sock) = start_daemon(dir.path());

    // —— 校验与加载 ——
    let decl_file = dir.path().join("proj.pact");
    std::fs::write(&decl_file, DECL).unwrap();
    // 文件不存在 → §6.3 错误体，exit 1
    let out = pact(&sock, true, &["domain", "validate", "nonexistent.pact"]);
    assert!(!out.status.success());
    let err: Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(err["error"]["code"], "Io");

    let v = ok_json(&pact(
        &sock,
        true,
        &["domain", "validate", decl_file.to_str().unwrap()],
    ));
    assert_eq!(v["ok"], true);
    let summary = ok_json(&pact(
        &sock,
        true,
        &[
            "domain",
            "load",
            decl_file.to_str().unwrap(),
            "--name",
            "proj",
        ],
    ));
    assert_eq!(summary["name"], "proj");
    assert_eq!(summary["types"], 3);

    // —— Rule-S（含错误路径：非授权参与者） ——
    let out = pact(
        &sock,
        true,
        &[
            "seed",
            "--domain",
            "proj",
            "--participant",
            "architect",
            "--type",
            "requirement",
        ],
    );
    assert!(!out.status.success());
    let err: Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(err["error"]["code"], "SeedAuthorityDenied");
    let seed = ok_json(&pact(
        &sock,
        true,
        &[
            "seed",
            "--domain",
            "proj",
            "--participant",
            "external",
            "--type",
            "requirement",
            "--thing-ref",
            "REQ-1",
        ],
    ));
    let req_id = seed["token"]["id"].as_str().unwrap().to_owned();

    // —— Rule-W（enabled）与 Rule-P ——
    let enabled = ok_json(&pact(
        &sock,
        true,
        &["enabled", "--domain", "proj", "--participant", "architect"],
    ));
    assert_eq!(enabled["options"][0]["type"], "design");
    let prod = ok_json(&pact(
        &sock,
        true,
        &[
            "produce",
            "--domain",
            "proj",
            "--participant",
            "architect",
            "--type",
            "design",
            "--predecessors",
            &req_id,
        ],
    ));
    let design_id = prod["token"]["id"].as_str().unwrap().to_owned();
    assert_ne!(design_id, req_id);

    // —— Rule-R / Rule-E ——
    let read = ok_json(&pact(
        &sock,
        true,
        &[
            "read",
            "--domain",
            "proj",
            "--participant",
            "architect",
            "--token",
            &req_id,
        ],
    ));
    assert_eq!(read["token"]["id"], req_id);
    let entry = ok_json(&pact(
        &sock,
        true,
        &[
            "consume",
            "--domain",
            "proj",
            "--participant",
            "developer",
            "--token",
            &design_id,
        ],
    ));
    assert_eq!(entry["entry"]["token"]["id"], design_id);
    assert_eq!(entry["entry"]["exhausted_by"], "developer");

    // —— 查询 ——
    let counter = ok_json(&pact(&sock, true, &["counter", "ls", "--domain", "proj"]));
    assert_eq!(counter["size"], 1);
    let archive = ok_json(&pact(&sock, true, &["archive", "ls", "--domain", "proj"]));
    assert_eq!(archive["size"], 1);
    let gates = ok_json(&pact(
        &sock,
        true,
        &[
            "gates",
            "show",
            "--domain",
            "proj",
            "--participant",
            "developer",
        ],
    ));
    assert_eq!(gates["gates"].as_array().unwrap().len(), 2);
    let status = ok_json(&pact(&sock, true, &["status", "--domain", "proj"]));
    assert_eq!(status["counter_size"], 1);
    assert_eq!(status["archive_size"], 1);

    // —— 人类模式冒烟 ——
    let out = pact(&sock, false, &["status", "--domain", "proj"]);
    assert!(out.status.success());
    let human = String::from_utf8_lossy(&out.stdout);
    assert!(human.contains("terminated="), "人类输出: {human}");

    // —— 离线 replay 对照：daemon 产出的事件日志重放后状态一致 ——
    // 域双键（D27）：日志落在域子目录下的 default 实例文件
    let log = dir
        .path()
        .join("data")
        .join("proj")
        .join("default.events.jsonl");
    assert!(log.exists(), "事件日志应存在");
    let replay = ok_json(
        &Command::new(env!("CARGO_BIN_EXE_pact"))
            .arg("--json")
            .args(["replay"])
            .arg(&log)
            .output()
            .unwrap(),
    );
    assert_eq!(replay["counter"].as_object().unwrap().len(), 1);
    assert_eq!(replay["archive"].as_array().unwrap().len(), 1);

    // —— 日志压缩（D25 子项 1）：截断前缀后状态不变、水位单调 ——
    let compacted = ok_json(&pact(&sock, true, &["compact", "--domain", "proj"]));
    assert_eq!(compacted["instance"], "default");
    assert_eq!(compacted["base_seq"], 4);
    assert_eq!(compacted["discarded_events"], 4);
    let status = ok_json(&pact(&sock, true, &["status", "--domain", "proj"]));
    assert_eq!(status["counter_size"], 1);
    assert_eq!(status["archive_size"], 1);
    let out = pact(&sock, false, &["compact", "--domain", "proj"]);
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("base_seq=4"),
        "人类输出: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    daemon.kill().unwrap();
    let _ = daemon.wait();
}

#[test]
fn offline_commands_without_daemon() {
    let dir = tempfile::tempdir().unwrap();

    // —— analyze（离线，不经 daemon） ——
    let decl_file = dir.path().join("lifecycle.pact");
    std::fs::write(&decl_file, DECL).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_pact"))
        .arg("--json")
        .args(["analyze"])
        .arg(&decl_file)
        .output()
        .unwrap();
    let report = ok_json(&out);
    assert!(report["reachability"]["unreachable"]
        .as_array()
        .unwrap()
        .is_empty());
    // A.5 报告为三元结构（well_formed / violations / warnings；
    // 结构体字段序上置于首位，JSON 键序由 serde_json 决定，不在此断言）
    assert_eq!(
        report["precondition_wf"]["well_formed"], true,
        "A.5 报告应良构"
    );
    assert!(report["precondition_wf"]["violations"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(report["precondition_wf"]["warnings"]
        .as_array()
        .unwrap()
        .is_empty());
    let out = Command::new(env!("CARGO_BIN_EXE_pact"))
        .args(["analyze"])
        .arg(&decl_file)
        .output()
        .unwrap();
    assert!(out.status.success());
    let human = String::from_utf8_lossy(&out.stdout);
    assert!(human.contains("precondition_wf: 良构"), "人类输出: {human}");
    assert!(human.contains("reachability"));

    // —— compose（离线，≥2 片段；片段为完整四段声明） ——
    let frag1 = dir.path().join("frag1.pact");
    let frag2 = dir.path().join("frag2.pact");
    std::fs::write(
        &frag1,
        "types { a; b }\npreconditions { a → b }\ngates { p: consumes a, produces b }\nseed_authority { e: seeds a }",
    )
    .unwrap();
    // frag2 与 frag1 共享类型 b（D18：单侧缺席前置无约束，取并合法）
    std::fs::write(
        &frag2,
        "types { b }\npreconditions { }\ngates { q: reads b }\nseed_authority { }",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_pact"))
        .arg("--json")
        .args(["compose"])
        .arg(&frag1)
        .arg(&frag2)
        .output()
        .unwrap();
    let composed = ok_json(&out);
    assert_eq!(composed["types"], 2);
    // gates 按 (参与者, 动作, 类型) 子句计数：p-consumes-a、p-produces-b、q-reads-b
    assert_eq!(composed["gates"], 3);
    // 人类模式输出完整合并声明
    let out = Command::new(env!("CARGO_BIN_EXE_pact"))
        .args(["compose"])
        .arg(&frag1)
        .arg(&frag2)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("seed_authority"));

    // —— cpn（离线：CPN Tools XML 生成；单实例命名空间） ——
    let cpn_file = dir.path().join("model.cpn");
    let out = Command::new(env!("CARGO_BIN_EXE_pact"))
        .arg("--json")
        .args(["cpn"])
        .arg(&decl_file)
        .arg("--out")
        .arg(&cpn_file)
        .args(["--instance", "alpha"])
        .output()
        .unwrap();
    let stats = ok_json(&out);
    assert_eq!(stats["places"], 4); // 3 类型 + Archive
    assert_eq!(stats["read_transitions"], 1);
    assert_eq!(stats["consume_transitions"], 3);
    assert_eq!(stats["production_transitions"], 2);
    assert_eq!(stats["seed_transitions"], 1);
    let xml = std::fs::read_to_string(&cpn_file).unwrap();
    assert!(xml.contains("<workspaceElements>"));
    assert!(xml.contains("i_alpha__p_requirement"));
    assert!(xml.contains("PACT_ARCHIVE"));

    // 缺失上界：`not exists` 必须显式要求 --bound，而非静默编码
    let neg = dir.path().join("neg.pact");
    std::fs::write(
        &neg,
        "types { a; b }\npreconditions { a → b }\ngates { p: reads b when not exists a }\nseed_authority { s: seeds a }",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_pact"))
        .arg("--json")
        .args(["cpn"])
        .arg(&neg)
        .arg("--out")
        .arg(dir.path().join("neg.cpn"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(err["error"]["code"], "Cpn");
    assert!(err["error"]["message"]
        .as_str()
        .unwrap()
        .contains("--bound"));

    // —— compose 冲突路径（C2：共享类型 b 双侧前置规则不同） ——
    let bad = dir.path().join("frag-bad.pact");
    std::fs::write(
        &bad,
        "types { b; c }\npreconditions { c → b }\ngates { }\nseed_authority { }",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_pact"))
        .arg("--json")
        .args(["compose"])
        .arg(&frag1)
        .arg(&bad)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err: Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(err["error"]["code"], "ComposeConflict");

    // —— replay：不存在的日志 → Io 错误 ——
    let out = Command::new(env!("CARGO_BIN_EXE_pact"))
        .arg("--json")
        .args(["replay", "/nonexistent/x.events.jsonl"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err: Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(err["error"]["code"], "Io");
}

/// D27：实例级命令与默认回退（CLI 侧）。
#[test]
fn instance_commands_and_default_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let (mut daemon, sock) = start_daemon(dir.path());

    let decl_file = dir.path().join("proj.pact");
    std::fs::write(&decl_file, DECL).unwrap();
    // 旧用法（无 --instance）落地 default 实例
    let summary = ok_json(&pact(
        &sock,
        true,
        &[
            "domain",
            "load",
            decl_file.to_str().unwrap(),
            "--name",
            "proj",
        ],
    ));
    assert_eq!(summary["instance"], "default");

    // 域清单含实例维度
    let list = ok_json(&pact(&sock, true, &["domain", "list"]));
    assert_eq!(list["domains"][0]["name"], "proj");
    assert_eq!(
        list["domains"][0]["instances"],
        serde_json::json!(["default"])
    );

    // 新增实例 x + instances 子命令
    let inst = ok_json(&pact(
        &sock,
        true,
        &[
            "domain",
            "instantiate",
            "--domain",
            "proj",
            "--instance",
            "x",
        ],
    ));
    assert_eq!(inst["instance"], "x");
    let instances = ok_json(&pact(
        &sock,
        true,
        &["domain", "instances", "--domain", "proj"],
    ));
    assert_eq!(instances, serde_json::json!(["default", "x"]));

    // 默认回退：不带 --instance 的 seed 落到 default
    ok_json(&pact(
        &sock,
        true,
        &[
            "seed",
            "--domain",
            "proj",
            "--participant",
            "external",
            "--type",
            "requirement",
        ],
    ));
    // 实例级 seed：x 两份
    for _ in 0..2 {
        ok_json(&pact(
            &sock,
            true,
            &[
                "seed",
                "--domain",
                "proj",
                "--instance",
                "x",
                "--participant",
                "external",
                "--type",
                "requirement",
            ],
        ));
    }

    // Counter 隔离
    let c_def = ok_json(&pact(&sock, true, &["counter", "ls", "--domain", "proj"]));
    assert_eq!(c_def["size"], 1);
    let c_x = ok_json(&pact(
        &sock,
        true,
        &["counter", "ls", "--domain", "proj", "--instance", "x"],
    ));
    assert_eq!(c_x["size"], 2);

    // 非空实例再 load 无 --force → DomainBusy（409）
    let out = pact(
        &sock,
        true,
        &[
            "domain",
            "load",
            decl_file.to_str().unwrap(),
            "--name",
            "proj",
            "--instance",
            "x",
        ],
    );
    assert!(!out.status.success());
    let err: Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(err["error"]["code"], "DomainBusy");

    // --force 重置该实例
    ok_json(&pact(
        &sock,
        true,
        &[
            "domain",
            "load",
            decl_file.to_str().unwrap(),
            "--name",
            "proj",
            "--instance",
            "x",
            "--force",
        ],
    ));
    let c_x = ok_json(&pact(
        &sock,
        true,
        &["counter", "ls", "--domain", "proj", "--instance", "x"],
    ));
    assert_eq!(c_x["size"], 0, "force 重置 x");
    let c_def = ok_json(&pact(&sock, true, &["counter", "ls", "--domain", "proj"]));
    assert_eq!(c_def["size"], 1, "default 不受影响");

    daemon.kill().unwrap();
    let _ = daemon.wait();
}

/// D48：真实二进制 `pact ref reload [--dry-run] [--json]` 与 daemon 联调。
#[test]
fn ref_reload_through_real_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let bindings = dir.path().join("refs.yaml");
    std::fs::write(
        &bindings,
        "apiVersion: pact.ref/v1\nkind: RefBindings\nmetadata:\n  domain: proj\nspec: {}\n",
    )
    .unwrap();
    let extra = format!("[ref_bindings]\npaths = [\"{}\"]\n", bindings.display());
    let (mut daemon, sock) = start_daemon_with(dir.path(), &extra);

    let decl_file = dir.path().join("proj.pact");
    std::fs::write(&decl_file, DECL).unwrap();
    ok_json(&pact(
        &sock,
        true,
        &[
            "domain",
            "load",
            decl_file.to_str().unwrap(),
            "--name",
            "proj",
        ],
    ));

    // 换绑：requirement 写入即要求钉版；dry-run 只报告不生效。
    std::fs::write(
        &bindings,
        "apiVersion: pact.ref/v1\nkind: RefBindings\nmetadata:\n  domain: proj\n\
         spec:\n  types:\n    requirement:\n      ref:\n        required: true\n",
    )
    .unwrap();
    let dry = ok_json(&pact(&sock, true, &["ref", "reload", "--dry-run"]));
    assert_eq!(dry["dryRun"], true);
    assert_eq!(dry["applied"], false);
    assert_eq!(dry["bindings"], 1);

    // dry-run 后旧规则仍生效：无 thing_ref 的 seed 成功。
    ok_json(&pact(
        &sock,
        true,
        &[
            "seed",
            "--domain",
            "proj",
            "--participant",
            "external",
            "--type",
            "requirement",
        ],
    ));

    let applied = ok_json(&pact(&sock, true, &["ref", "reload"]));
    assert_eq!(applied["dryRun"], false);
    assert_eq!(applied["applied"], true);
    assert_eq!(applied["bindings"], 1);

    // 新规则生效：缺失 thing_ref → ThingRefRequired。
    let out = pact(
        &sock,
        true,
        &[
            "seed",
            "--domain",
            "proj",
            "--participant",
            "external",
            "--type",
            "requirement",
        ],
    );
    assert!(!out.status.success());
    let err: Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(err["error"]["code"], "ThingRefRequired");

    // 审计：每次尝试一行。
    let text = std::fs::read_to_string(dir.path().join("data").join("ref-reload.jsonl")).unwrap();
    assert_eq!(text.lines().count(), 2);

    daemon.kill().unwrap();
    let _ = daemon.wait();
}
