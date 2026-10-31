//! `pact ref lint` 离线端到端：真实二进制、真实文件、目录展开与退出码。

use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};

const DECL: &str = r#"
types { requirement; design; implementation }
preconditions { requirement → design; design → implementation }
gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: consumes design, produces implementation;
    user: consumes implementation
}
seed_authority { external: seeds requirement }
"#;

const BINDINGS: &str = r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: se_lifecycle
  declarationDigest: sha256:19fe1f29f567a5314572ba8af957129e9570cff3de6ca3bc0da9523441bdcbfb
spec:
  types:
    design:
      ref:
        required: true
        schemes: [https]
        contract: se/design-review@2
        pinning: required-at-archive
"#;

const CARD: &str = r#"
apiVersion: pact.ref/v1
kind: ContractCard
metadata:
  id: se/design-review
  version: "2"
  owner: architecture-guild
  status: active
spec:
  artifact:
    kind: document
    mediaType: text/markdown
    digestPolicy: optional
  reference:
    schemes: [https]
    pinning:
      requiredForArchive: true
  lifecycle:
    retention: P365D
    deletionPolicy: tombstone-then-delete
    tombstoneRef: urn:pact:tombstone:se/design-review
"#;

const TOMBSTONE: &str = r#"
apiVersion: pact.ref/v1
kind: Tombstone
metadata:
  id: "urn:pact:tombstone:se/design-review:req-001"
spec:
  contract: se/design-review@2
  ref: https://artifacts.example.com/design/req-001
  reason: expired
  deletedAt: 2026-10-27T00:00:00Z
  retainUntil: 2027-10-27T00:00:00Z
"#;

fn pact(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pact"))
        .args(args)
        .output()
        .expect("执行 pact 命令失败")
}

fn report(out: &Output) -> Value {
    serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("输出应为合法 JSON")
}

fn write(dir: &Path, name: &str, text: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path.display().to_string()
}

#[test]
fn ref_lint_static_checks() {
    let dir = tempfile::tempdir().unwrap();
    let declaration = write(dir.path(), "se_lifecycle.pact", DECL);
    let bindings = write(dir.path(), "se_lifecycle.refs.yaml", BINDINGS);
    let contracts_dir = dir.path().join("contracts");
    std::fs::create_dir(&contracts_dir).unwrap();
    std::fs::write(contracts_dir.join("design.yaml"), CARD).unwrap();
    let tombstones_dir = dir.path().join("tombstones");
    std::fs::create_dir(&tombstones_dir).unwrap();
    std::fs::write(tombstones_dir.join("req-001.yaml"), TOMBSTONE).unwrap();

    // 一致输入：目录展开 ContractCard / Tombstone，0 error / 0 warning。
    let out = pact(&[
        "--json",
        "ref",
        "lint",
        "--bindings",
        &bindings,
        "--contracts",
        contracts_dir.to_str().unwrap(),
        "--tombstones",
        tombstones_dir.to_str().unwrap(),
        "--declaration",
        &declaration,
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value = report(&out);
    assert_eq!(value["bindings"], 1);
    assert_eq!(value["contracts"], 1);
    assert_eq!(value["tombstones"], 1);
    assert_eq!(value["errors"], 0);
    assert_eq!(value["warnings"], 0, "report: {value}");
    assert!(value["findings"].as_array().unwrap().is_empty());

    // 类型拼写错误 → error，退出 1。
    let bad = write(
        dir.path(),
        "bad.refs.yaml",
        &BINDINGS.replace("    design:", "    desgin:"),
    );
    let out = pact(&[
        "--json",
        "ref",
        "lint",
        "--bindings",
        &bad,
        "--contracts",
        contracts_dir.to_str().unwrap(),
        "--declaration",
        &declaration,
    ]);
    assert!(!out.status.success());
    let value = report(&out);
    assert_eq!(value["errors"], 1);
    assert_eq!(value["findings"][0]["code"], "ref.bindings.unknown_type");

    // 未提供 ContractCard：仅警告，退出 0；--strict 视为失败。
    let out = pact(&[
        "--json",
        "ref",
        "lint",
        "--bindings",
        &bindings,
        "--declaration",
        &declaration,
    ]);
    assert!(out.status.success());
    let value = report(&out);
    assert_eq!(value["errors"], 0);
    assert!(value["warnings"].as_u64().unwrap() > 0);
    assert!(value["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["code"] == "ref.bindings.contract_unverified"));

    let out = pact(&[
        "--json",
        "ref",
        "lint",
        "--bindings",
        &bindings,
        "--declaration",
        &declaration,
        "--strict",
    ]);
    assert!(!out.status.success(), "--strict 应把 warning 视为失败");

    // 人类模式摘要。
    let out = pact(&[
        "ref",
        "lint",
        "--bindings",
        &bindings,
        "--contracts",
        contracts_dir.to_str().unwrap(),
        "--tombstones",
        tombstones_dir.to_str().unwrap(),
        "--declaration",
        &declaration,
    ]);
    assert!(out.status.success());
    let human = String::from_utf8_lossy(&out.stdout);
    assert!(
        human.contains("bindings=1 contracts=1 tombstones=1 errors=0 warnings=0"),
        "人类输出: {human}"
    );

    // 坏 YAML → 解析错误，不中断其余检查。
    let broken = write(dir.path(), "broken.yaml", "apiVersion: [");
    let out = pact(&[
        "--json",
        "ref",
        "lint",
        "--bindings",
        &broken,
        "--declaration",
        &declaration,
    ]);
    assert!(!out.status.success());
    let value = report(&out);
    assert_eq!(value["findings"][0]["code"], "ref.bindings.parse");

    // 不存在的 bindings 路径 → Io 错误体。
    let out = pact(&[
        "--json",
        "ref",
        "lint",
        "--bindings",
        "/nonexistent/refs.yaml",
    ]);
    assert!(!out.status.success());
    let value = report(&out);
    assert_eq!(value["error"]["code"], "Io");
}

/// D49 第一步：`pact ref digest` 与 lint 漂移比对走真实二进制。
#[test]
fn ref_digest_and_lint_drift() {
    let dir = tempfile::tempdir().unwrap();
    let declaration = write(dir.path(), "se_lifecycle.pact", DECL);

    let out = pact(&["--json", "ref", "digest", &declaration]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value = report(&out);
    assert_eq!(value["algorithm"], "sha256");
    assert_eq!(value["digestVersion"], 1);
    assert_eq!(
        value["declarationDigest"],
        "sha256:19fe1f29f567a5314572ba8af957129e9570cff3de6ca3bc0da9523441bdcbfb"
    );

    // 人类模式只打印摘要。
    let out = pact(&["ref", "digest", &declaration]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "sha256:19fe1f29f567a5314572ba8af957129e9570cff3de6ca3bc0da9523441bdcbfb"
    );

    // 域名参与摘要：显式 --name 改变结果。
    let out = pact(&[
        "--json",
        "ref",
        "digest",
        &declaration,
        "--name",
        "other_domain",
    ]);
    assert!(out.status.success());
    let value = report(&out);
    assert_ne!(
        value["declarationDigest"],
        "sha256:19fe1f29f567a5314572ba8af957129e9570cff3de6ca3bc0da9523441bdcbfb"
    );

    // 非法域名 → Config 错误。
    let out = pact(&[
        "--json",
        "ref",
        "digest",
        &declaration,
        "--name",
        "bad.name",
    ]);
    assert!(!out.status.success());
    assert_eq!(report(&out)["error"]["code"], "Config");

    // 不一致 → error 且退出 1。
    let mismatched = write(
        dir.path(),
        "mismatched.refs.yaml",
        &BINDINGS.replace(
            "sha256:19fe1f29f567a5314572ba8af957129e9570cff3de6ca3bc0da9523441bdcbfb",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ),
    );
    let out = pact(&[
        "--json",
        "ref",
        "lint",
        "--bindings",
        &mismatched,
        "--declaration",
        &declaration,
    ]);
    assert!(!out.status.success(), "digest 不一致应使 lint 失败");
    let value = report(&out);
    assert_eq!(value["errors"], 1);
    assert!(value["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["code"] == "ref.bindings.digest_mismatch"));

    // 缺失仍是 warning：默认退出 0，--strict 退出 1。
    let missing = write(
        dir.path(),
        "missing.refs.yaml",
        &BINDINGS.replace(
            "  declarationDigest: sha256:19fe1f29f567a5314572ba8af957129e9570cff3de6ca3bc0da9523441bdcbfb\n",
            "",
        ),
    );
    let out = pact(&[
        "--json",
        "ref",
        "lint",
        "--bindings",
        &missing,
        "--declaration",
        &declaration,
    ]);
    assert!(out.status.success());
    let value = report(&out);
    assert_eq!(value["errors"], 0);
    assert!(value["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["code"] == "ref.bindings.digest_missing"));
    let out = pact(&[
        "--json",
        "ref",
        "lint",
        "--bindings",
        &missing,
        "--declaration",
        &declaration,
        "--strict",
    ]);
    assert!(!out.status.success(), "--strict 应把缺失 warning 视为失败");
}

const DECL_WITHOUT_DESIGN_READER: &str = r#"
types { requirement; design; implementation }
preconditions { requirement → design; design → implementation }
gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: produces implementation;
    user: consumes implementation
}
seed_authority { external: seeds requirement }
"#;

const CARD_WITHOUT_LIFECYCLE: &str = r#"
apiVersion: pact.ref/v1
kind: ContractCard
metadata:
  id: se/design-review
  version: "2"
  owner: architecture-guild
  status: active
spec:
  artifact:
    kind: document
    mediaType: text/markdown
    digestPolicy: optional
  reference:
    schemes: [https]
"#;

/// 可发现性检查走真实二进制：读取路径缺失、保留证据缺失都要出现在 JSON 报告中。
#[test]
fn ref_lint_reports_discoverability_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let declaration = write(dir.path(), "se_lifecycle.pact", DECL_WITHOUT_DESIGN_READER);
    let bindings = write(
        dir.path(),
        "se_lifecycle.refs.yaml",
        &BINDINGS.replace(
            "sha256:19fe1f29f567a5314572ba8af957129e9570cff3de6ca3bc0da9523441bdcbfb",
            // 该测试的声明缺 design 读路径，摘要与主 fixture 不同。
            "sha256:c8edd150985d3c2e9ab836df2ad59e685e892bea5981e5145993c7b58a4f8934",
        ),
    );
    let card = write(dir.path(), "design.yaml", CARD_WITHOUT_LIFECYCLE);

    let out = pact(&[
        "--json",
        "ref",
        "lint",
        "--bindings",
        &bindings,
        "--contracts",
        &card,
        "--declaration",
        &declaration,
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value = report(&out);
    assert_eq!(value["errors"], 0, "report: {value}");
    let codes: Vec<&str> = value["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|finding| finding["code"].as_str())
        .collect();
    assert!(
        codes.contains(&"ref.discoverability.view_read_path_missing"),
        "findings: {codes:?}"
    );
    assert!(
        codes.contains(&"ref.discoverability.no_reader"),
        "findings: {codes:?}"
    );
    assert!(
        codes.contains(&"ref.discoverability.retention_missing"),
        "findings: {codes:?}"
    );

    let out = pact(&[
        "ref",
        "lint",
        "--bindings",
        &bindings,
        "--contracts",
        &card,
        "--declaration",
        &declaration,
        "--strict",
    ]);
    assert!(
        !out.status.success(),
        "--strict 应把可发现性 warning 视为失败"
    );
}
