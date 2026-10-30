//! D48：RefBindings 与 ContractCard 的显式重载端到端。
//!
//! 覆盖 `dryRun` 影响报告、原子替换、失败保持旧状态、审计落盘、ACL 与认证；
//! 不做文件监听/自动回滚/按域灰度。

use pact_engine::{Engine, EngineConfig, LogicalClock, SequentialIdGen};
use pact_http::client::{Client, ClientError};
use pact_http::config::AclMap;
use pact_http::reload::RefReloadConfig;
use pact_http::server::{router, serve_unix, ConnTracker, ServerState};
use pact_http::visibility::VisibilityProfiles;
use pact_store::Store;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

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

/// 最小绑定：域 `proj`，无钉版要求。
const PLAIN_BINDINGS: &str = r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: proj
spec: {}
"#;

/// 换绑到更严规则：`design` 归档前必须钉版。
const ARCHIVE_PIN_BINDINGS: &str = r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: proj
spec:
  types:
    design:
      ref:
        pinning: required-at-archive
"#;

/// 声明另一个域的绑定（用于 ACL 覆盖新索引的用例）。
const OTHER_DOMAIN_BINDINGS: &str = r#"
apiVersion: pact.ref/v1
kind: RefBindings
metadata:
  domain: other
spec: {}
"#;

const INVALID_BINDINGS: &str = "apiVersion: pact.ref/v1\nkind: RefBindings\nmetadata: [\n";

fn tokens() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("external".to_owned(), "ext-token".to_owned()),
        ("architect".to_owned(), "arch-token".to_owned()),
        ("developer".to_owned(), "dev-token".to_owned()),
    ])
}

fn binding_path(dir: &Path) -> PathBuf {
    dir.join("refs.yaml")
}

/// 启动 daemon；`audit_is_dir = true` 时把审计目标做成目录以触发写失败。
#[allow(clippy::too_many_arguments)]
async fn spawn(
    dir: &Path,
    tokens: BTreeMap<String, String>,
    acl: AclMap,
    initial_bindings: &str,
    audit_is_dir: bool,
    configure_reload: bool,
) -> PathBuf {
    let bindings = binding_path(dir);
    std::fs::write(&bindings, initial_bindings).unwrap();
    let profiles = VisibilityProfiles::load(std::slice::from_ref(&bindings)).unwrap();
    if audit_is_dir {
        std::fs::create_dir_all(dir.join("data").join("ref-reload.jsonl")).unwrap();
    }
    let store = Store::open(dir.join("data")).await.unwrap();
    let engine = Engine::spawn_with_config(
        store,
        Arc::new(LogicalClock::new()),
        Arc::new(SequentialIdGen::new()),
        EngineConfig::default(),
    );
    let state = ServerState::with_identity_and_visibility(
        engine,
        tokens,
        BTreeMap::new(),
        acl,
        None,
        None,
        profiles,
    );
    if configure_reload {
        state.configure_reload(RefReloadConfig::new(
            vec![bindings],
            Vec::new(),
            dir.join("data"),
        ));
    }
    let app = router(state);
    let sock = dir.join("pact.sock");
    let server_sock = sock.clone();
    tokio::spawn(async move {
        let _ = serve_unix(
            app,
            &server_sock,
            watch::channel(false).1,
            ConnTracker::default(),
        )
        .await;
    });
    for _ in 0..100 {
        if tokio::net::UnixStream::connect(&sock).await.is_ok() {
            return sock;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("daemon 套接字未就绪");
}

async fn seed_requirement(c: &Client) {
    c.seed(
        "proj",
        None,
        "external",
        "requirement",
        Some("https://example.com/req"),
    )
    .await
    .unwrap();
}

async fn produce_design(c: &Client, requirement_id: &str) -> String {
    c.produce(
        "proj",
        None,
        "architect",
        "design",
        vec![requirement_id.to_owned()],
        Some("https://example.com/design"),
    )
    .await
    .unwrap()
    .token
    .id
    .0
}

/// D48：`dryRun` 报告新增阻断数但不替换；确认后才生效。
#[tokio::test]
async fn dry_run_reports_newly_blocked_and_confirmation_applies() {
    let dir = tempfile::tempdir().unwrap();
    let sock = spawn(
        dir.path(),
        tokens(),
        BTreeMap::new(),
        PLAIN_BINDINGS,
        false,
        true,
    )
    .await;
    let external = Client::unix(&sock).with_bearer("ext-token");
    let architect = Client::unix(&sock).with_bearer("arch-token");
    let developer = Client::unix(&sock).with_bearer("dev-token");
    external
        .load_domain("proj", DECL, None, false)
        .await
        .unwrap();
    seed_requirement(&external).await;
    let requirement = external
        .counter("proj", None, Some("requirement"))
        .await
        .unwrap()
        .tokens[0]
        .id
        .0
        .clone();
    let design = produce_design(&architect, &requirement).await;

    // 换绑到更严规则。
    std::fs::write(binding_path(dir.path()), ARCHIVE_PIN_BINDINGS).unwrap();
    let report = architect.reload(true).await.unwrap();
    assert!(report.dry_run);
    assert!(!report.applied, "dryRun 不替换索引");
    assert_eq!(report.bindings, 1);
    assert_eq!(report.contracts, 0);
    assert_eq!(report.blocked_total, 1, "存量未钉版 design 将不可归档");
    assert_eq!(report.blocked_tokens.len(), 1);
    assert_eq!(report.blocked_tokens[0].domain, "proj");
    assert_eq!(report.blocked_tokens[0].instance, "default");
    assert_eq!(report.blocked_tokens[0].ty, "design");
    assert_eq!(report.blocked_tokens[0].tokens, 1);

    // 旧规则仍生效：未钉版 design 可正常归档。
    developer
        .consume("proj", None, "developer", &design, None)
        .await
        .unwrap();

    // 再造一个未钉版 design，确认后应被新规则阻断。
    let design = produce_design(&architect, &requirement).await;
    let applied = architect.reload(false).await.unwrap();
    assert!(!applied.dry_run);
    assert!(applied.applied);
    assert_eq!(applied.blocked_total, 1);
    match developer
        .consume("proj", None, "developer", &design, None)
        .await
    {
        Err(ClientError::Api(detail)) => assert_eq!(detail.code, "ThingRefUnpinned"),
        other => panic!("重载后应按新规则阻断未钉版归档，实际 {other:?}"),
    }

    // 审计：两次尝试各一行，含源文件摘要与结果。
    let text = std::fs::read_to_string(dir.path().join("data").join("ref-reload.jsonl")).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["dryRun"], true);
    assert_eq!(lines[0]["outcome"], "applied");
    assert_eq!(lines[0]["by"], "architect");
    assert_eq!(lines[0]["bindings"], 1);
    assert_eq!(lines[0]["blockedTokens"][0]["tokens"], 1);
    let files = lines[0]["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["kind"], "bindings");
    assert_eq!(files[0]["sha256"].as_str().unwrap().len(), 64);
    assert_eq!(lines[1]["dryRun"], false);
    assert_eq!(lines[1]["outcome"], "applied");
}

/// D48：解析/校验失败 → `400 RefReloadInvalid`，旧快照继续生效，审计记 rejected。
#[tokio::test]
async fn invalid_reload_keeps_old_profile() {
    let dir = tempfile::tempdir().unwrap();
    let sock = spawn(
        dir.path(),
        tokens(),
        BTreeMap::new(),
        PLAIN_BINDINGS,
        false,
        true,
    )
    .await;
    let external = Client::unix(&sock).with_bearer("ext-token");
    let architect = Client::unix(&sock).with_bearer("arch-token");
    let developer = Client::unix(&sock).with_bearer("dev-token");
    external
        .load_domain("proj", DECL, None, false)
        .await
        .unwrap();
    seed_requirement(&external).await;
    let requirement = external
        .counter("proj", None, Some("requirement"))
        .await
        .unwrap()
        .tokens[0]
        .id
        .0
        .clone();
    let design = produce_design(&architect, &requirement).await;

    std::fs::write(binding_path(dir.path()), INVALID_BINDINGS).unwrap();
    match architect.reload(false).await {
        Err(ClientError::Api(detail)) => assert_eq!(detail.code, "RefReloadInvalid"),
        other => panic!("非法绑定应以 400 拒绝，实际 {other:?}"),
    }

    // 旧快照仍生效：未钉版 design 可归档。
    developer
        .consume("proj", None, "developer", &design, None)
        .await
        .unwrap();

    audit_lines(dir.path(), 1, |line| {
        assert_eq!(line["outcome"], "rejected");
        assert!(line["error"].as_str().unwrap().contains("refs.yaml"));
    });
}

fn audit_lines(dir: &Path, expected: usize, check: impl Fn(&serde_json::Value)) {
    let text = std::fs::read_to_string(dir.join("data").join("ref-reload.jsonl")).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), expected);
    check(&lines[lines.len() - 1]);
}

/// D48：审计写失败 → `500 RefReloadAuditFailed`，不替换索引。
#[tokio::test]
async fn audit_write_failure_rejects_reload() {
    let dir = tempfile::tempdir().unwrap();
    let sock = spawn(
        dir.path(),
        tokens(),
        BTreeMap::new(),
        PLAIN_BINDINGS,
        true,
        true,
    )
    .await;
    let external = Client::unix(&sock).with_bearer("ext-token");
    let architect = Client::unix(&sock).with_bearer("arch-token");
    let developer = Client::unix(&sock).with_bearer("dev-token");
    external
        .load_domain("proj", DECL, None, false)
        .await
        .unwrap();
    seed_requirement(&external).await;
    let requirement = external
        .counter("proj", None, Some("requirement"))
        .await
        .unwrap()
        .tokens[0]
        .id
        .0
        .clone();
    let design = produce_design(&architect, &requirement).await;

    std::fs::write(binding_path(dir.path()), ARCHIVE_PIN_BINDINGS).unwrap();
    match architect.reload(false).await {
        Err(ClientError::Api(detail)) => assert_eq!(detail.code, "RefReloadAuditFailed"),
        other => panic!("审计失败应拒绝重载，实际 {other:?}"),
    }
    // 旧规则仍生效，未钉版 design 可归档。
    developer
        .consume("proj", None, "developer", &design, None)
        .await
        .unwrap();
}

/// D48：配置身份后必须认证；信任声明模式放行匿名。
#[tokio::test]
async fn reload_requires_authentication_in_token_mode() {
    let dir = tempfile::tempdir().unwrap();
    let sock = spawn(
        dir.path(),
        tokens(),
        BTreeMap::new(),
        PLAIN_BINDINGS,
        false,
        true,
    )
    .await;
    match Client::unix(&sock).reload(true).await {
        Err(ClientError::Api(detail)) => assert_eq!(detail.code, "Unauthorized"),
        other => panic!("令牌模式匿名重载应 401，实际 {other:?}"),
    }

    let dir = tempfile::tempdir().unwrap();
    let sock = spawn(
        dir.path(),
        BTreeMap::new(),
        BTreeMap::new(),
        PLAIN_BINDINGS,
        false,
        true,
    )
    .await;
    let report = Client::unix(&sock).reload(true).await.unwrap();
    assert!(report.dry_run);
    assert_eq!(report.bindings, 1);
}

/// D48：ACL 启用时，重载须覆盖**新索引**中的每个绑定域（`{domain}/*`）。
#[tokio::test]
async fn reload_requires_wildcard_acl_for_new_bindings_domains() {
    let dir = tempfile::tempdir().unwrap();
    let acl = AclMap::from([("architect".to_owned(), vec!["proj/*".to_owned()])]);
    let sock = spawn(
        dir.path(),
        BTreeMap::from([("architect".to_owned(), "arch-token".to_owned())]),
        acl,
        PLAIN_BINDINGS,
        false,
        true,
    )
    .await;
    let architect = Client::unix(&sock).with_bearer("arch-token");

    std::fs::write(binding_path(dir.path()), OTHER_DOMAIN_BINDINGS).unwrap();
    match architect.reload(false).await {
        Err(ClientError::Api(detail)) => assert_eq!(detail.code, "Forbidden"),
        other => panic!("新域未授权应以 403 拒绝，实际 {other:?}"),
    }
    audit_lines(dir.path(), 1, |line| {
        assert_eq!(line["outcome"], "rejected")
    });

    // 失败不替换：新索引仍是 proj，再次重载依旧被新域授权拒绝。
    match architect.reload(true).await {
        Err(ClientError::Api(detail)) => assert_eq!(detail.code, "Forbidden"),
        other => panic!("新域未授权应持续拒绝，实际 {other:?}"),
    }
}

/// D48：ACL 也覆盖当前索引，只给单实例授权不足。
#[tokio::test]
async fn reload_requires_wildcard_acl_for_current_domain() {
    let dir = tempfile::tempdir().unwrap();
    let acl = AclMap::from([("architect".to_owned(), vec!["proj/default".to_owned()])]);
    let sock = spawn(
        dir.path(),
        BTreeMap::from([("architect".to_owned(), "arch-token".to_owned())]),
        acl,
        PLAIN_BINDINGS,
        false,
        true,
    )
    .await;
    let architect = Client::unix(&sock).with_bearer("arch-token");
    match architect.reload(true).await {
        Err(ClientError::Api(detail)) => assert_eq!(detail.code, "Forbidden"),
        other => panic!("单实例授权不足以重载，实际 {other:?}"),
    }
}

/// D48：未注入重载配置时返回结构化 500，不静默成功。
#[tokio::test]
async fn reload_unavailable_without_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let sock = spawn(
        dir.path(),
        BTreeMap::new(),
        BTreeMap::new(),
        PLAIN_BINDINGS,
        false,
        false,
    )
    .await;
    match Client::unix(&sock).reload(true).await {
        Err(ClientError::Api(detail)) => assert_eq!(detail.code, "RefReloadUnavailable"),
        other => panic!("未配置重载源应返回结构化错误，实际 {other:?}"),
    }
}
