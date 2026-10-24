//! P3 金标对账：WASM 内存宿主 vs 原生引擎。
//!
//! 覆盖：事件序列逐行相等、视图相等、Rule-T 终止事件、错误码、
//! 幂等重放、JSONL 双向互通（WASM 日志 → 原生 daemon；原生日志 → WASM restore）、
//! 快照导出与恢复。固定 id/ts：原生用 `SequentialIdGen` + `LogicalClock`，
//! WASM 侧显式注入同一序列。

use pact_core::{ArchiveEntry, ParticipantName, RuleError, ThingRef, Token, TokenId, TypeName};
use pact_engine::{Engine, EngineError, InstanceKey, LogicalClock, SequentialIdGen};
use pact_store::Store;
use pact_wasm::PactHost;
use serde_json::json;
use std::sync::Arc;

/// 三段生命周期域：requirement --architect--> design --developer--> implementation。
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

fn par(s: &str) -> ParticipantName {
    ParticipantName(s.into())
}
fn tyn(s: &str) -> TypeName {
    TypeName(s.into())
}

fn token_of(value: &serde_json::Value) -> Token {
    serde_json::from_value(value.clone()).unwrap()
}
fn entry_of(value: &serde_json::Value) -> ArchiveEntry {
    serde_json::from_value(value.clone()).unwrap()
}

async fn native_engine(dir: &std::path::Path) -> Engine {
    let store = Store::open(dir).await.unwrap();
    Engine::spawn(
        store,
        Arc::new(LogicalClock::new()),
        Arc::new(SequentialIdGen::new()),
    )
}

/// 走完整生命周期（含一次幂等重放与 Rule-T 终止），返回两套宿主。
async fn run_corpus() -> (
    tempfile::TempDir,
    Engine,
    InstanceKey,
    PactHost,
    InstanceKey,
) {
    let dir = tempfile::tempdir().unwrap();
    let engine = native_engine(dir.path()).await;
    let nk = InstanceKey::new("pipeline", "default");
    engine
        .load_domain("pipeline", DECL, "default", false)
        .await
        .unwrap();

    let mut host = PactHost::new();
    host.load_domain_at("pipeline", DECL, "default", "T0", false)
        .unwrap();
    let wk = PactHost::parse_key("pipeline/default").unwrap();

    // T1 seed requirement
    let n_req = engine
        .seed(
            nk.clone(),
            par("external"),
            tyn("requirement"),
            ThingRef::some("urn:req:1"),
        )
        .await
        .unwrap();
    let p = host
        .prepare_seed(
            &wk,
            "external",
            "requirement",
            Some("urn:req:1"),
            "seq-0",
            "T1",
        )
        .unwrap();
    assert_eq!(token_of(&p.result), n_req);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    // Rule-R（无事件、不消耗时钟）
    let n_read = engine
        .read(nk.clone(), par("architect"), n_req.id.clone())
        .await
        .unwrap();
    assert_eq!(host.read(&wk, "architect", "seq-0").unwrap(), n_read);

    // T2 produce design
    let n_design = engine
        .produce(
            nk.clone(),
            par("architect"),
            tyn("design"),
            vec![n_req.id.clone()],
            ThingRef::some("urn:design:1"),
        )
        .await
        .unwrap();
    let p = host
        .prepare_produce(
            &wk,
            "architect",
            "design",
            &json!(["seq-0"]).to_string(),
            Some("urn:design:1"),
            "seq-1",
            "T2",
        )
        .unwrap();
    assert_eq!(token_of(&p.result), n_design);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    // T3 consume requirement（rid-1）
    let n_entry = engine
        .consume(
            nk.clone(),
            par("architect"),
            n_req.id.clone(),
            Some("rid-1".into()),
        )
        .await
        .unwrap();
    let p = host
        .prepare_consume(&wk, "architect", "seq-0", Some("rid-1"), "T3")
        .unwrap();
    assert_eq!(entry_of(&p.result), n_entry);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    // T4 幂等重放：无事件、返回首次归档结果
    let n_again = engine
        .consume(
            nk.clone(),
            par("architect"),
            n_req.id.clone(),
            Some("rid-1".into()),
        )
        .await
        .unwrap();
    let w_again = host
        .prepare_consume(&wk, "architect", "seq-0", Some("rid-1"), "T4")
        .unwrap();
    assert!(w_again.replayed);
    assert!(w_again.tx_id.is_none());
    assert_eq!(entry_of(&w_again.result), n_again);

    // T5 produce implementation
    let n_impl = engine
        .produce(
            nk.clone(),
            par("developer"),
            tyn("implementation"),
            vec![n_design.id.clone()],
            ThingRef::some("urn:impl:1"),
        )
        .await
        .unwrap();
    let p = host
        .prepare_produce(
            &wk,
            "developer",
            "implementation",
            &json!(["seq-1"]).to_string(),
            Some("urn:impl:1"),
            "seq-2",
            "T5",
        )
        .unwrap();
    assert_eq!(token_of(&p.result), n_impl);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    // T6 consume design（rid-2）
    let n_entry = engine
        .consume(
            nk.clone(),
            par("developer"),
            n_design.id.clone(),
            Some("rid-2".into()),
        )
        .await
        .unwrap();
    let p = host
        .prepare_consume(&wk, "developer", "seq-1", Some("rid-2"), "T6")
        .unwrap();
    assert_eq!(entry_of(&p.result), n_entry);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    // T7 consume implementation → Counter 空 → Rule-T 在 T8 追加 Terminated
    let n_entry = engine
        .consume(nk.clone(), par("user"), n_impl.id.clone(), None)
        .await
        .unwrap();
    let p = host
        .prepare_consume(&wk, "user", "seq-2", None, "T7")
        .unwrap();
    assert_eq!(entry_of(&p.result), n_entry);
    let committed = host
        .commit(p.tx_id.as_deref().unwrap(), Some("T8"))
        .unwrap();
    assert!(committed.terminated.is_some());
    assert_eq!(committed.terminated.unwrap().seq, 8);

    (dir, engine, nk, host, wk)
}

#[tokio::test]
async fn golden_events_views_and_jsonl_interop() {
    let (dir, engine, nk, host, wk) = run_corpus().await;

    // 1) 事件序列逐行相等
    let native_log =
        std::fs::read_to_string(dir.path().join("pipeline/default.events.jsonl")).unwrap();
    let wasm_log = host.export_events(&wk).unwrap();
    assert_eq!(
        wasm_log.lines().collect::<Vec<_>>(),
        native_log.lines().collect::<Vec<_>>(),
        "WASM 事件序列应与原生逐行一致"
    );

    // 2) 视图相等
    assert_eq!(
        host.status(&wk).unwrap(),
        engine.status(nk.clone()).await.unwrap()
    );
    assert_eq!(
        host.counter(&wk, None).unwrap(),
        engine.counter(nk.clone(), None).await.unwrap()
    );
    assert_eq!(
        host.archive(&wk, None).unwrap(),
        engine.archive(nk.clone(), None).await.unwrap()
    );
    for p in ["architect", "developer", "user"] {
        assert_eq!(
            host.gates(&wk, p).unwrap(),
            engine.my_gates(nk.clone(), par(p)).await.unwrap()
        );
        assert_eq!(
            host.enabled(&wk, p).unwrap(),
            engine.enabled(nk.clone(), par(p)).await.unwrap()
        );
    }

    // 3) 原生日志 → WASM restore
    let mut restored = PactHost::new();
    let st = restored
        .restore("pipeline", "default", &native_log, None)
        .unwrap();
    assert_eq!(st, host.status(&wk).unwrap());
    assert_eq!(
        restored.counter(&wk, None).unwrap(),
        host.counter(&wk, None).unwrap()
    );
    assert_eq!(
        restored.archive(&wk, None).unwrap(),
        host.archive(&wk, None).unwrap()
    );
    assert_eq!(restored.export_events(&wk).unwrap(), wasm_log);

    // 4) 原生日志 + WASM 快照 → 新宿主（快照快速路径）
    let snap = serde_json::to_string(&host.export_snapshot(&wk).unwrap()).unwrap();
    let mut from_snap = PactHost::new();
    assert_eq!(
        from_snap
            .restore("pipeline", "default", &native_log, Some(&snap))
            .unwrap(),
        host.status(&wk).unwrap()
    );

    // 5) WASM 日志（+快照）→ 原生 daemon 加载
    let dir2 = tempfile::tempdir().unwrap();
    let instance_dir = dir2.path().join("pipeline");
    std::fs::create_dir_all(&instance_dir).unwrap();
    std::fs::write(instance_dir.join("default.events.jsonl"), &wasm_log).unwrap();
    std::fs::write(instance_dir.join("default.snapshot.json"), &snap).unwrap();
    let engine2 = native_engine(dir2.path()).await;
    assert_eq!(
        engine2.status(nk.clone()).await.unwrap(),
        host.status(&wk).unwrap()
    );
    assert_eq!(
        engine2.counter(nk.clone(), None).await.unwrap(),
        host.counter(&wk, None).unwrap()
    );
    assert_eq!(
        engine2.archive(nk.clone(), None).await.unwrap(),
        host.archive(&wk, None).unwrap()
    );
}

#[tokio::test]
async fn error_codes_match_native() {
    let dir = tempfile::tempdir().unwrap();
    let engine = native_engine(dir.path()).await;
    let nk = InstanceKey::new("pipeline", "default");
    engine
        .load_domain("pipeline", DECL, "default", false)
        .await
        .unwrap();

    let mut host = PactHost::new();
    host.load_domain_at("pipeline", DECL, "default", "T0", false)
        .unwrap();
    let wk = PactHost::parse_key("pipeline/default").unwrap();

    // 未授权种子
    let ne = engine
        .seed(
            nk.clone(),
            par("nobody"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap_err();
    let we = host
        .prepare_seed(&wk, "nobody", "requirement", None, "seq-0", "T1")
        .unwrap_err();
    assert_eq!(we.code, native_rule_code(&ne));

    // 前驱不在 Counter
    let ne = engine
        .produce(
            nk.clone(),
            par("architect"),
            tyn("design"),
            vec![TokenId("missing".into())],
            ThingRef::none(),
        )
        .await
        .unwrap_err();
    let we = host
        .prepare_produce(
            &wk,
            "architect",
            "design",
            &json!(["missing"]).to_string(),
            None,
            "seq-0",
            "T2",
        )
        .unwrap_err();
    assert_eq!(we.code, native_rule_code(&ne));

    // 目标 token 不在 Counter
    let ne = engine
        .consume(
            nk.clone(),
            par("architect"),
            TokenId("missing".into()),
            None,
        )
        .await
        .unwrap_err();
    let we = host
        .prepare_consume(&wk, "architect", "missing", None, "T3")
        .unwrap_err();
    assert_eq!(we.code, native_rule_code(&ne));

    // 实例非空时未 force 重载
    let n_req = engine
        .seed(
            nk.clone(),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    let p = host
        .prepare_seed(&wk, "external", "requirement", None, "seq-0", "T4")
        .unwrap();
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();
    let ne = engine
        .load_domain("pipeline", DECL, "default", false)
        .await
        .unwrap_err();
    let we = host
        .load_domain_at("pipeline", DECL, "default", "T5", false)
        .unwrap_err();
    assert_eq!(we.code, native_code(&ne));
    assert_eq!(we.code, "DomainBusy");

    // 幂等键冲突：同 rid 指向另一 token
    let n_req2 = engine
        .seed(
            nk.clone(),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    let p = host
        .prepare_seed(&wk, "external", "requirement", None, "seq-1", "T6")
        .unwrap();
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();
    engine
        .consume(
            nk.clone(),
            par("architect"),
            n_req.id.clone(),
            Some("rid-x".into()),
        )
        .await
        .unwrap();
    let p = host
        .prepare_consume(&wk, "architect", "seq-0", Some("rid-x"), "T7")
        .unwrap();
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();
    let ne = engine
        .consume(
            nk.clone(),
            par("architect"),
            n_req2.id.clone(),
            Some("rid-x".into()),
        )
        .await
        .unwrap_err();
    let we = host
        .prepare_consume(&wk, "architect", "seq-1", Some("rid-x"), "T8")
        .unwrap_err();
    assert_eq!(we.code, native_code(&ne));
    assert_eq!(we.code, "IdempotencyConflict");
}

/// P4 退出条件：原生引擎与 WASM 宿主可交替打开同一 JSONL 日志。
#[tokio::test]
async fn native_and_wasm_alternate_on_shared_log() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("pipeline/default.events.jsonl");
    let nk = InstanceKey::new("pipeline", "default");
    let wk = PactHost::parse_key("pipeline/default").unwrap();

    // 原生 A：建实例 + 种子 + 生产
    let engine_a = native_engine(dir.path()).await;
    engine_a
        .load_domain("pipeline", DECL, "default", false)
        .await
        .unwrap();
    let req = engine_a
        .seed(
            nk.clone(),
            par("external"),
            tyn("requirement"),
            ThingRef::some("urn:req:1"),
        )
        .await
        .unwrap();
    engine_a
        .produce(
            nk.clone(),
            par("architect"),
            tyn("design"),
            vec![req.id.clone()],
            ThingRef::some("urn:design:1"),
        )
        .await
        .unwrap();

    // WASM B：接力日志 → 消费 requirement → 覆写日志
    let mut host = PactHost::new();
    host.restore(
        "pipeline",
        "default",
        &std::fs::read_to_string(&log_path).unwrap(),
        None,
    )
    .unwrap();
    let p = host
        .prepare_consume(&wk, "architect", &req.id.0, Some("rid-shared"), "T9")
        .unwrap();
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();
    std::fs::write(&log_path, host.export_events(&wk).unwrap()).unwrap();

    // 原生 C：打开 WASM 写过的日志 → 视图一致 → 再追加种子
    // 顺序 id 重启后会从 0 重来；日志中已有 seq-0/seq-1，必须换起点避免撞号。
    let engine_c = Engine::spawn(
        Store::open(dir.path()).await.unwrap(),
        Arc::new(LogicalClock::new()),
        Arc::new(SequentialIdGen::new_at(1000)),
    );
    assert_eq!(
        engine_c.status(nk.clone()).await.unwrap(),
        host.status(&wk).unwrap()
    );
    assert_eq!(
        engine_c.counter(nk.clone(), None).await.unwrap(),
        host.counter(&wk, None).unwrap()
    );
    assert_eq!(
        engine_c.archive(nk.clone(), None).await.unwrap(),
        host.archive(&wk, None).unwrap()
    );
    engine_c
        .seed(
            nk.clone(),
            par("external"),
            tyn("requirement"),
            ThingRef::some("urn:req:2"),
        )
        .await
        .unwrap();

    // WASM D：再次打开原生追加后的日志
    let mut host2 = PactHost::new();
    host2
        .restore(
            "pipeline",
            "default",
            &std::fs::read_to_string(&log_path).unwrap(),
            None,
        )
        .unwrap();
    assert_eq!(host2.status(&wk).unwrap().counter_size, 2);
    assert_eq!(host2.archive(&wk, None).unwrap().size, 1);
}

/// Q7 后的金标扩面 1：同一域声明并发承载多个实例，事件与视图按实例隔离。
#[tokio::test]
async fn multi_instance_isolation_matches_native() {
    let dir = tempfile::tempdir().unwrap();
    let engine = native_engine(dir.path()).await;
    engine
        .load_domain("shared", DECL, "default", false)
        .await
        .unwrap();
    let summary = engine.instantiate("shared", "second").await.unwrap();
    assert_eq!(summary.last_seq, 1, "新实例从独立 DomainLoaded 起算");
    let nk_default = InstanceKey::new("shared", "default");
    let nk_second = InstanceKey::new("shared", "second");

    let mut host = PactHost::new();
    host.load_domain_at("shared", DECL, "default", "T0", false)
        .unwrap();
    assert_eq!(host.instantiate("shared", "second", "T1").unwrap(), summary);
    let wk_default = PactHost::parse_key("shared/default").unwrap();
    let wk_second = PactHost::parse_key("shared/second").unwrap();

    // 两个实例各自种子（原生 idgen 全局递增，WASM 显式注入同一序列）
    let n_req = engine
        .seed(
            nk_default.clone(),
            par("external"),
            tyn("requirement"),
            ThingRef::some("urn:req:default"),
        )
        .await
        .unwrap();
    let p = host
        .prepare_seed(
            &wk_default,
            "external",
            "requirement",
            Some("urn:req:default"),
            "seq-0",
            "T2",
        )
        .unwrap();
    assert_eq!(token_of(&p.result), n_req);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    let n_req2 = engine
        .seed(
            nk_second.clone(),
            par("external"),
            tyn("requirement"),
            ThingRef::some("urn:req:second"),
        )
        .await
        .unwrap();
    let p = host
        .prepare_seed(
            &wk_second,
            "external",
            "requirement",
            Some("urn:req:second"),
            "seq-1",
            "T3",
        )
        .unwrap();
    assert_eq!(token_of(&p.result), n_req2);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    // 消耗 default 实例的 token，second 实例不受影响
    let n_entry = engine
        .consume(
            nk_default.clone(),
            par("architect"),
            n_req.id.clone(),
            Some("rid-multi".into()),
        )
        .await
        .unwrap();
    let p = host
        .prepare_consume(&wk_default, "architect", "seq-0", Some("rid-multi"), "T4")
        .unwrap();
    assert_eq!(entry_of(&p.result), n_entry);
    let committed = host
        .commit(p.tx_id.as_deref().unwrap(), Some("T5"))
        .unwrap();
    assert!(committed.terminated.is_some());

    // 每个实例的事件日志与原生逐行一致
    for (instance, key) in [("default", &wk_default), ("second", &wk_second)] {
        let native_log = std::fs::read_to_string(
            dir.path()
                .join("shared")
                .join(format!("{instance}.events.jsonl")),
        )
        .unwrap();
        assert_eq!(
            host.export_events(key).unwrap().lines().collect::<Vec<_>>(),
            native_log.lines().collect::<Vec<_>>(),
            "实例 {instance} 的事件序列应与原生一致"
        );
        assert_eq!(
            host.status(key).unwrap(),
            engine
                .status(if instance == "default" {
                    nk_default.clone()
                } else {
                    nk_second.clone()
                })
                .await
                .unwrap()
        );
    }

    // 隔离断言：second 仍有 1 枚 token，archive 为空
    assert_eq!(host.counter(&wk_second, None).unwrap().size, 1);
    assert_eq!(host.archive(&wk_second, None).unwrap().size, 0);
    assert_eq!(host.counter(&wk_default, None).unwrap().size, 0);
    assert_eq!(host.archive(&wk_default, None).unwrap().size, 1);

    let summary = host
        .list_domains()
        .into_iter()
        .find(|d| d.name == "shared")
        .unwrap();
    assert_eq!(
        summary.instances,
        vec!["default".to_owned(), "second".to_owned()]
    );
}

/// Q7 后的金标扩面 2：片段组合（§1.8）后的事件序列与视图与原生一致。
#[tokio::test]
async fn composed_domain_matches_native() {
    const FRAGMENT_A: &str = r#"
types { implementation; review }
preconditions { implementation → review }
gates {
    impl_owner: reads implementation, produces review;
    reviewer: reads review, consumes review
}
seed_authority { external: seeds implementation }
"#;
    const FRAGMENT_B: &str = r#"
types { implementation; security_report }
preconditions { implementation → security_report }
gates {
    scanner: reads implementation, produces security_report;
    auditor: reads security_report, consumes security_report
}
seed_authority { }
"#;

    let dir = tempfile::tempdir().unwrap();
    let engine = native_engine(dir.path()).await;
    let n_summary = engine
        .compose_and_load(
            "merged",
            vec![FRAGMENT_A.to_owned(), FRAGMENT_B.to_owned()],
            "default",
        )
        .await
        .unwrap();
    let nk = InstanceKey::new("merged", "default");

    let mut host = PactHost::new();
    let w_summary = host
        .compose(
            "merged",
            &json!([FRAGMENT_A, FRAGMENT_B]).to_string(),
            "default",
            "T0",
        )
        .unwrap();
    assert_eq!(w_summary, n_summary);
    let wk = PactHost::parse_key("merged/default").unwrap();

    let n_impl = engine
        .seed(
            nk.clone(),
            par("external"),
            tyn("implementation"),
            ThingRef::some("urn:impl:1"),
        )
        .await
        .unwrap();
    let p = host
        .prepare_seed(
            &wk,
            "external",
            "implementation",
            Some("urn:impl:1"),
            "seq-0",
            "T1",
        )
        .unwrap();
    assert_eq!(token_of(&p.result), n_impl);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    let n_review = engine
        .produce(
            nk.clone(),
            par("impl_owner"),
            tyn("review"),
            vec![n_impl.id.clone()],
            ThingRef::some("urn:review:1"),
        )
        .await
        .unwrap();
    let p = host
        .prepare_produce(
            &wk,
            "impl_owner",
            "review",
            &json!(["seq-0"]).to_string(),
            Some("urn:review:1"),
            "seq-1",
            "T2",
        )
        .unwrap();
    assert_eq!(token_of(&p.result), n_review);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    let n_report = engine
        .produce(
            nk.clone(),
            par("scanner"),
            tyn("security_report"),
            vec![n_impl.id.clone()],
            ThingRef::some("urn:sec:1"),
        )
        .await
        .unwrap();
    let p = host
        .prepare_produce(
            &wk,
            "scanner",
            "security_report",
            &json!(["seq-0"]).to_string(),
            Some("urn:sec:1"),
            "seq-2",
            "T3",
        )
        .unwrap();
    assert_eq!(token_of(&p.result), n_report);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    for participant in ["impl_owner", "reviewer", "scanner", "auditor"] {
        assert_eq!(
            host.gates(&wk, participant).unwrap(),
            engine.my_gates(nk.clone(), par(participant)).await.unwrap(),
            "参与者 {participant} 的门应一致"
        );
    }

    let native_log =
        std::fs::read_to_string(dir.path().join("merged/default.events.jsonl")).unwrap();
    assert_eq!(
        host.export_events(&wk).unwrap().lines().collect::<Vec<_>>(),
        native_log.lines().collect::<Vec<_>>(),
        "组合域的事件序列应与原生一致"
    );
    assert_eq!(
        host.counter(&wk, None).unwrap(),
        engine.counter(nk.clone(), None).await.unwrap()
    );
}

/// Q7 后的金标扩面 3：GateExpr 条件分支（count / not exists）准入与拒绝逐项对账。
#[tokio::test]
async fn conditional_gate_branches_match_native() {
    const COND_DECL: &str = r#"
types { task; extra; report }
preconditions { task → report }
gates {
    worker: reads task when count task >= 2,
            consumes task when count extra >= 1,
            produces report;
    reviewer: reads report when not exists extra, consumes report;
    admin: consumes extra
}
seed_authority { admin: seeds task; admin: seeds extra }
"#;

    let dir = tempfile::tempdir().unwrap();
    let engine = native_engine(dir.path()).await;
    engine
        .load_domain("cond", COND_DECL, "default", false)
        .await
        .unwrap();
    let nk = InstanceKey::new("cond", "default");

    let mut host = PactHost::new();
    host.load_domain_at("cond", COND_DECL, "default", "T0", false)
        .unwrap();
    let wk = PactHost::parse_key("cond/default").unwrap();

    let n_task = engine
        .seed(
            nk.clone(),
            par("admin"),
            tyn("task"),
            ThingRef::some("urn:task:1"),
        )
        .await
        .unwrap();
    let p = host
        .prepare_seed(&wk, "admin", "task", Some("urn:task:1"), "seq-0", "T1")
        .unwrap();
    assert_eq!(token_of(&p.result), n_task);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    // count task >= 2：只有 1 枚 → 读被拒
    let ne = engine
        .read(nk.clone(), par("worker"), n_task.id.clone())
        .await
        .unwrap_err();
    let we = host.read(&wk, "worker", "seq-0").unwrap_err();
    assert_eq!(we.code, native_rule_code(&ne), "count 条件拒绝码应一致");
    assert_eq!(we.code, "GateConditionFailed");

    let n_task2 = engine
        .seed(
            nk.clone(),
            par("admin"),
            tyn("task"),
            ThingRef::some("urn:task:2"),
        )
        .await
        .unwrap();
    let p = host
        .prepare_seed(&wk, "admin", "task", Some("urn:task:2"), "seq-1", "T2")
        .unwrap();
    assert_eq!(token_of(&p.result), n_task2);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    // 2 枚 → 读放行
    assert_eq!(
        host.read(&wk, "worker", "seq-0").unwrap(),
        engine
            .read(nk.clone(), par("worker"), n_task.id.clone())
            .await
            .unwrap()
    );

    // count extra >= 1：未注入 → 消费被拒
    let ne = engine
        .consume(nk.clone(), par("worker"), n_task.id.clone(), None)
        .await
        .unwrap_err();
    let we = host
        .prepare_consume(&wk, "worker", "seq-0", None, "T3")
        .unwrap_err();
    assert_eq!(we.code, native_rule_code(&ne));

    let n_extra = engine
        .seed(
            nk.clone(),
            par("admin"),
            tyn("extra"),
            ThingRef::some("urn:extra:1"),
        )
        .await
        .unwrap();
    let p = host
        .prepare_seed(&wk, "admin", "extra", Some("urn:extra:1"), "seq-2", "T4")
        .unwrap();
    assert_eq!(token_of(&p.result), n_extra);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    let n_entry = engine
        .consume(
            nk.clone(),
            par("worker"),
            n_task.id.clone(),
            Some("rid-c".into()),
        )
        .await
        .unwrap();
    let p = host
        .prepare_consume(&wk, "worker", "seq-0", Some("rid-c"), "T5")
        .unwrap();
    assert_eq!(entry_of(&p.result), n_entry);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    let n_report = engine
        .produce(
            nk.clone(),
            par("worker"),
            tyn("report"),
            vec![n_task2.id.clone()],
            ThingRef::some("urn:report:1"),
        )
        .await
        .unwrap();
    let p = host
        .prepare_produce(
            &wk,
            "worker",
            "report",
            &json!(["seq-1"]).to_string(),
            Some("urn:report:1"),
            "seq-3",
            "T6",
        )
        .unwrap();
    assert_eq!(token_of(&p.result), n_report);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    // not exists extra：extra 在场 → 读被拒
    let ne = engine
        .read(nk.clone(), par("reviewer"), n_report.id.clone())
        .await
        .unwrap_err();
    let we = host.read(&wk, "reviewer", "seq-3").unwrap_err();
    assert_eq!(we.code, native_rule_code(&ne));

    let n_entry = engine
        .consume(nk.clone(), par("admin"), n_extra.id.clone(), None)
        .await
        .unwrap();
    let p = host
        .prepare_consume(&wk, "admin", "seq-2", None, "T7")
        .unwrap();
    assert_eq!(entry_of(&p.result), n_entry);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    // extra 消失 → 读放行，随后消费 report
    assert_eq!(
        host.read(&wk, "reviewer", "seq-3").unwrap(),
        engine
            .read(nk.clone(), par("reviewer"), n_report.id.clone())
            .await
            .unwrap()
    );
    let n_entry = engine
        .consume(nk.clone(), par("reviewer"), n_report.id.clone(), None)
        .await
        .unwrap();
    let p = host
        .prepare_consume(&wk, "reviewer", "seq-3", None, "T8")
        .unwrap();
    assert_eq!(entry_of(&p.result), n_entry);
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();

    let native_log = std::fs::read_to_string(dir.path().join("cond/default.events.jsonl")).unwrap();
    assert_eq!(
        host.export_events(&wk).unwrap().lines().collect::<Vec<_>>(),
        native_log.lines().collect::<Vec<_>>(),
        "条件分支的事件序列应与原生一致"
    );
    assert_eq!(
        host.status(&wk).unwrap(),
        engine.status(nk.clone()).await.unwrap()
    );
    assert_eq!(
        host.counter(&wk, None).unwrap(),
        engine.counter(nk.clone(), None).await.unwrap()
    );
    assert_eq!(
        host.archive(&wk, None).unwrap(),
        engine.archive(nk.clone(), None).await.unwrap()
    );
}

/// Q7 后的金标扩面 4：原生压缩后的「快照 + 空日志」可由 WASM 恢复并继续写入，
/// 且原生 daemon 能再打开 WASM 追加的尾部日志。
#[tokio::test]
async fn compacted_log_recovery_matches_native() {
    let dir = tempfile::tempdir().unwrap();
    let engine = native_engine(dir.path()).await;
    let nk = InstanceKey::new("pipeline", "default");
    engine
        .load_domain("pipeline", DECL, "default", false)
        .await
        .unwrap();
    let req = engine
        .seed(
            nk.clone(),
            par("external"),
            tyn("requirement"),
            ThingRef::some("urn:req:1"),
        )
        .await
        .unwrap();
    engine
        .produce(
            nk.clone(),
            par("architect"),
            tyn("design"),
            vec![req.id.clone()],
            ThingRef::some("urn:design:1"),
        )
        .await
        .unwrap();
    engine
        .consume(nk.clone(), par("architect"), req.id.clone(), None)
        .await
        .unwrap();
    engine.compact(nk.clone(), 0).await.unwrap();

    let log_path = dir.path().join("pipeline/default.events.jsonl");
    let snap_path = dir.path().join("pipeline/default.snapshot.json");
    let compacted_log = std::fs::read_to_string(&log_path).unwrap();
    let snapshot = std::fs::read_to_string(&snap_path).unwrap();
    assert!(
        compacted_log.trim().is_empty(),
        "keep_last=0 时日志应被清空，实际：{compacted_log:?}"
    );

    let mut host = PactHost::new();
    assert_eq!(
        host.restore("pipeline", "default", &compacted_log, Some(&snapshot))
            .unwrap(),
        engine.status(nk.clone()).await.unwrap()
    );
    let wk = PactHost::parse_key("pipeline/default").unwrap();
    assert_eq!(
        host.counter(&wk, None).unwrap(),
        engine.counter(nk.clone(), None).await.unwrap()
    );
    assert_eq!(
        host.archive(&wk, None).unwrap(),
        engine.archive(nk.clone(), None).await.unwrap()
    );

    // WASM 在快照之后续写：seq 必须接续快照水位，日志只含尾部事件
    let p = host
        .prepare_seed(
            &wk,
            "external",
            "requirement",
            Some("urn:req:tail"),
            "tail-0",
            "T4",
        )
        .unwrap();
    host.commit(p.tx_id.as_deref().unwrap(), None).unwrap();
    let tail = host.export_events(&wk).unwrap();
    assert_eq!(tail.lines().count(), 1, "压缩后导出应只含快照之后的尾部");
    let tail_event: serde_json::Value = serde_json::from_str(tail.trim()).unwrap();
    assert_eq!(tail_event["seq"], json!(5));
    std::fs::write(&log_path, &tail).unwrap();

    drop(engine);
    let reopened = native_engine(dir.path()).await;
    assert_eq!(
        reopened.status(nk.clone()).await.unwrap(),
        host.status(&wk).unwrap()
    );
    assert_eq!(
        reopened.counter(nk.clone(), None).await.unwrap(),
        host.counter(&wk, None).unwrap()
    );
    assert_eq!(
        reopened.archive(nk.clone(), None).await.unwrap(),
        host.archive(&wk, None).unwrap()
    );
}

fn native_rule_code(err: &EngineError) -> String {
    match err {
        EngineError::Rule(e) => match e {
            RuleError::SeedAuthorityDenied { .. } => "SeedAuthorityDenied".into(),
            RuleError::PredecessorNotOnCounter { .. } => "PredecessorNotOnCounter".into(),
            RuleError::TokenNotOnCounter { .. } => "TokenNotOnCounter".into(),
            RuleError::GateConditionFailed { .. } => "GateConditionFailed".into(),
            RuleError::StaticGateDenied { .. } => "StaticGateDenied".into(),
            RuleError::DuplicateTokenId { .. } => "DuplicateTokenId".into(),
            other => panic!("未覆盖的规则错误：{other:?}"),
        },
        other => panic!("期望规则错误，实际：{other}"),
    }
}

fn native_code(err: &EngineError) -> String {
    match err {
        EngineError::DomainBusy { .. } => "DomainBusy".into(),
        EngineError::IdempotencyConflict { .. } => "IdempotencyConflict".into(),
        EngineError::InstanceNotFound { .. } => "InstanceNotFound".into(),
        EngineError::InstanceExists { .. } => "InstanceExists".into(),
        EngineError::DomainNotDeclared { .. } => "DomainNotDeclared".into(),
        other => panic!("未覆盖的引擎错误：{other}"),
    }
}
