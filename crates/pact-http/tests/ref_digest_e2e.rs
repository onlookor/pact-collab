//! D49 第二步：运行期声明摘要比对（`[ref_digest] mode`）的端到端。
//!
//! 覆盖 `off` / `warn` / `enforce` 三模式、绑定未记录摘要、该域无绑定与
//! 声明解析失败的分支；`enforce` 拒绝时域不得被加载（无副作用）。

use pact_engine::{Engine, EngineConfig, LogicalClock, SequentialIdGen};
use pact_http::client::{Client, ClientError};
use pact_http::config::RefDigestMode;
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

/// 与 DECL 语义不同但同样良构：新增一个类型，摘要必然变化。
const DECL_CHANGED: &str = r#"
types { requirement; design; implementation; review }
preconditions { requirement → design; design → implementation }
gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: consumes design, produces implementation;
    user: consumes implementation
}
seed_authority { external: seeds requirement }
"#;

fn digest_of(domain: &str, declaration: &str) -> String {
    let ast = pact_parser::load_domain(declaration).unwrap();
    pact_analysis::digest::declaration_digest(domain, &ast)
}

fn bindings_with_digest(domain: &str, digest: Option<&str>) -> String {
    let digest_line = digest
        .map(|digest| format!("  declarationDigest: \"{digest}\"\n"))
        .unwrap_or_default();
    format!(
        "apiVersion: pact.ref/v1\nkind: RefBindings\nmetadata:\n  domain: {domain}\n{digest_line}spec: {{}}\n"
    )
}

async fn spawn(
    dir: &Path,
    bindings: Option<&str>,
    mode: RefDigestMode,
) -> (PathBuf, Arc<ServerState>) {
    let profiles = match bindings {
        Some(text) => {
            let path = dir.join("refs.yaml");
            std::fs::write(&path, text).unwrap();
            VisibilityProfiles::load(&[path]).unwrap()
        }
        None => VisibilityProfiles::empty(),
    };
    let store = Store::open(dir.join("data")).await.unwrap();
    let engine = Engine::spawn_with_config(
        store,
        Arc::new(LogicalClock::new()),
        Arc::new(SequentialIdGen::new()),
        EngineConfig::default(),
    );
    let state = ServerState::with_identity_and_visibility(
        engine,
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeMap::new(),
        None,
        None,
        profiles,
    );
    state.configure_ref_digest(mode);
    let app = router(state.clone());
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
            return (sock, state);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("daemon 套接字未就绪");
}

#[tokio::test]
async fn enforce_accepts_matching_digest() {
    let dir = tempfile::tempdir().unwrap();
    let expected = digest_of("proj", DECL);
    let bindings = bindings_with_digest("proj", Some(&expected));
    let (sock, _state) = spawn(dir.path(), Some(&bindings), RefDigestMode::Enforce).await;
    let client = Client::unix(sock);
    client
        .load_domain("proj", DECL, None, false)
        .await
        .expect("摘要一致时 enforce 应放行");
}

#[tokio::test]
async fn enforce_rejects_mismatch_without_loading() {
    let dir = tempfile::tempdir().unwrap();
    let recorded = digest_of("proj", DECL);
    let bindings = bindings_with_digest("proj", Some(&recorded));
    let (sock, _state) = spawn(dir.path(), Some(&bindings), RefDigestMode::Enforce).await;
    let client = Client::unix(sock);

    match client.load_domain("proj", DECL_CHANGED, None, false).await {
        Err(ClientError::Api(detail)) => {
            assert_eq!(detail.code, "DeclarationDigestMismatch");
            assert_eq!(detail.detail["expected"], recorded);
            assert_eq!(detail.detail["digestVersion"], 1);
            assert!(detail.detail["actual"].is_string());
        }
        other => panic!("预期 DeclarationDigestMismatch，实际 {other:?}"),
    }
    // 拒绝发生在进入引擎前：域未被登记。
    let list = client.list_domains().await.unwrap();
    assert!(list.domains.is_empty(), "enforce 拒绝后不得留下已加载域");
}

#[tokio::test]
async fn warn_allows_mismatch_and_off_skips_comparison() {
    let recorded = digest_of("proj", DECL);
    let bindings = bindings_with_digest("proj", Some(&recorded));

    let dir = tempfile::tempdir().unwrap();
    let (sock, _state) = spawn(dir.path(), Some(&bindings), RefDigestMode::Warn).await;
    let client = Client::unix(sock);
    client
        .load_domain("proj", DECL_CHANGED, None, false)
        .await
        .expect("warn 模式不一致时仍应加载");

    let dir = tempfile::tempdir().unwrap();
    let (sock, _state) = spawn(dir.path(), Some(&bindings), RefDigestMode::Off).await;
    let client = Client::unix(sock);
    client
        .load_domain("proj", DECL_CHANGED, None, false)
        .await
        .expect("off 模式不比较摘要");
}

#[tokio::test]
async fn missing_digest_or_foreign_domain_is_not_compared() {
    // 绑定未记录摘要 → enforce 也不比较。
    let dir = tempfile::tempdir().unwrap();
    let bindings = bindings_with_digest("proj", None);
    let (sock, _state) = spawn(dir.path(), Some(&bindings), RefDigestMode::Enforce).await;
    let client = Client::unix(sock);
    client
        .load_domain("proj", DECL_CHANGED, None, false)
        .await
        .expect("未记录摘要时不得拒绝加载");

    // 该域无绑定 → enforce 也不比较。
    let dir = tempfile::tempdir().unwrap();
    let bindings = bindings_with_digest("other", Some(&digest_of("other", DECL)));
    let (sock, _state) = spawn(dir.path(), Some(&bindings), RefDigestMode::Enforce).await;
    let client = Client::unix(sock);
    client
        .load_domain("proj", DECL_CHANGED, None, false)
        .await
        .expect("无绑定的域不得被摘要比对拒绝");
}

#[tokio::test]
async fn invalid_declaration_keeps_existing_error_code() {
    let dir = tempfile::tempdir().unwrap();
    let bindings = bindings_with_digest("proj", Some(&digest_of("proj", DECL)));
    let (sock, _state) = spawn(dir.path(), Some(&bindings), RefDigestMode::Enforce).await;
    let client = Client::unix(sock);
    match client.load_domain("proj", "types { a }", None, false).await {
        Err(ClientError::Api(detail)) => {
            assert!(
                matches!(detail.code.as_str(), "Syntax" | "WellFormedness"),
                "声明解析失败应保留既有错误码，实际 {}",
                detail.code
            );
        }
        other => panic!("预期声明错误，实际 {other:?}"),
    }
}
