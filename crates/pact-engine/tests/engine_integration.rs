//! pact-engine 集成测试（设计 §10 阶段 3 验收项）：
//! - 引擎命令流（多域、并发命令串行化）；
//! - 重放等价（快照快速路径 与 纯重放路径 状态一致）；
//! - 崩溃恢复（JSONL 尾部半行截断、索引丢失重建、中部损坏 fail-fast）；
//! - 订阅通知时序（事件顺序、seq 单调、变更后规模）；
//! - 域替换重置（恢复只重放最后一次 DomainLoaded 之后的事件）。
//!
//! 域双键（D27）：本文件全部以 `default` 实例（[`DEFAULT_INSTANCE`]）运行旧语义，
//! 存储布局为 `<data_dir>/<domain>/default.events.jsonl`。

use pact_core::{ParticipantName, RuleError, ThingRef, TokenId, TypeName};
use pact_engine::views::DomainListing;
use pact_engine::{
    Engine, EngineConfig, EngineError, InstanceKey, LogicalClock, NotificationKind,
    SequentialIdGen, DEFAULT_INSTANCE,
};
use pact_store::Store;
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

/// `default` 实例的双键（域双键 D27）。
fn key(domain: &str) -> InstanceKey {
    InstanceKey::new(domain, DEFAULT_INSTANCE)
}

/// 临时数据目录 + 引擎（LogicalClock + 顺序 id，确定性）。
async fn setup(dir: &std::path::Path) -> Engine {
    setup_at(dir, 0).await
}

/// 指定顺序 id 起点；重启后仍要写入非空实例时，起点须避开历史 id。
async fn setup_at(dir: &std::path::Path, id_start: u64) -> Engine {
    let store = Store::open(dir).await.unwrap();
    Engine::spawn(
        store,
        Arc::new(LogicalClock::new()),
        Arc::new(SequentialIdGen::new_at(id_start)),
    )
}

/// 模拟崩溃：丢弃句柄（不写终态快照），让 actor 排干通道后退出。
async fn crash(engine: Engine) {
    drop(engine);
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
}

/// `default` 实例的事件日志路径：`<dir>/<domain>/default.events.jsonl`。
fn log_path(dir: &std::path::Path, domain: &str) -> std::path::PathBuf {
    dir.join(domain)
        .join(format!("{DEFAULT_INSTANCE}.events.jsonl"))
}

/// `default` 实例的快照路径。
fn snapshot_path(dir: &std::path::Path, domain: &str) -> std::path::PathBuf {
    dir.join(domain)
        .join(format!("{DEFAULT_INSTANCE}.snapshot.json"))
}

fn log_lines(dir: &std::path::Path, domain: &str) -> Vec<String> {
    std::fs::read_to_string(log_path(dir, domain))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// 取域清单中的名字（排序）。
fn domain_names(listings: Vec<DomainListing>) -> Vec<String> {
    listings.into_iter().map(|d| d.name).collect()
}

/// seed → produce → consume 前进到"design 在 Counter、requirement 已归档"。
/// 事件序列：DomainLoaded(1) Seeded(2) Produced(3) Consumed(4)。
async fn run_partial_flow(engine: &Engine, domain: &str) -> TokenId {
    let k = key(domain);
    let req = engine
        .seed(
            k.clone(),
            par("external"),
            tyn("requirement"),
            ThingRef::some("urn:req:1"),
        )
        .await
        .unwrap();
    let design = engine
        .produce(
            k.clone(),
            par("architect"),
            tyn("design"),
            vec![req.id.clone()],
            ThingRef::some("urn:design:1"),
        )
        .await
        .unwrap();
    engine
        .consume(k, par("architect"), req.id.clone(), None)
        .await
        .unwrap();
    design.id
}

// ============ 1. 命令流 + 通知时序 + Rule-W 查询 ============

#[tokio::test]
async fn full_lifecycle_with_notifications() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;

    let summary = engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    assert_eq!((summary.types, summary.gates, summary.last_seq), (3, 6, 1));

    // Rule-W：种子注入前 architect 无可生产选项
    assert!(engine
        .enabled(key("proj"), par("architect"))
        .await
        .unwrap()
        .is_empty());

    // 订阅先于动作（broadcast 只投递给已存在的订阅者）
    let mut rx = engine.subscribe(key("proj")).await.unwrap();

    let req = engine
        .seed(
            key("proj"),
            par("external"),
            tyn("requirement"),
            ThingRef::some("urn:req:1"),
        )
        .await
        .unwrap();
    assert_eq!(req.produced_by, par("external"));

    // Rule-W：requirement 就位后 design 生产选项出现，见证前驱 = 该 token
    let enabled = engine.enabled(key("proj"), par("architect")).await.unwrap();
    assert_eq!(enabled.len(), 1);
    assert_eq!(enabled[0].ty, tyn("design"));
    assert_eq!(enabled[0].witness_predecessors, vec![req.id.clone()]);

    // my_gates 自省
    let gates = engine
        .my_gates(key("proj"), par("architect"))
        .await
        .unwrap();
    assert_eq!(gates.len(), 3);
    assert!(gates.iter().any(|g| g.ty == tyn("requirement")));
    assert!(gates.iter().any(|g| g.ty == tyn("design")));

    // Rule-R 无副作用（D9）：读前后日志行数不变
    let lines_before = log_lines(dir.path(), "proj").len();
    let read_back = engine
        .read(key("proj"), par("architect"), req.id.clone())
        .await
        .unwrap();
    assert_eq!(read_back.id, req.id);
    assert_eq!(log_lines(dir.path(), "proj").len(), lines_before);

    let design = engine
        .produce(
            key("proj"),
            par("architect"),
            tyn("design"),
            vec![req.id.clone()],
            ThingRef::some("urn:design:1"),
        )
        .await
        .unwrap();
    engine
        .consume(key("proj"), par("architect"), req.id.clone(), None)
        .await
        .unwrap();
    let impl_t = engine
        .produce(
            key("proj"),
            par("developer"),
            tyn("implementation"),
            vec![design.id.clone()],
            ThingRef::some("urn:impl:1"),
        )
        .await
        .unwrap();
    engine
        .consume(key("proj"), par("developer"), design.id.clone(), None)
        .await
        .unwrap();
    engine
        .consume(key("proj"), par("user"), impl_t.id.clone(), None)
        .await
        .unwrap();

    // 终态：Counter 空、Archive 3 项、Rule-T 成立
    let status = engine.status(key("proj")).await.unwrap();
    assert_eq!(
        (status.counter_size, status.archive_size, status.terminated),
        (0, 3, true)
    );
    assert_eq!(engine.counter(key("proj"), None).await.unwrap().size, 0);
    assert_eq!(
        engine
            .archive(key("proj"), Some(tyn("requirement")))
            .await
            .unwrap()
            .size,
        1
    );

    // 通知时序：种类、seq 单调、变更后 Counter 规模
    let expected = [
        (NotificationKind::Seeded, 2u64, 1usize),
        (NotificationKind::Produced, 3, 2),
        (NotificationKind::Consumed, 4, 1),
        (NotificationKind::Produced, 5, 2),
        (NotificationKind::Consumed, 6, 1),
        (NotificationKind::Consumed, 7, 0),
        (NotificationKind::Terminated, 8, 0),
    ];
    for (kind, seq, cs) in expected {
        let n = rx.try_recv().expect("通知应已缓冲");
        assert_eq!(n.event, kind, "通知顺序：{kind:?} @seq={seq}");
        assert_eq!(n.seq, seq);
        assert_eq!(n.summary.counter_size, cs);
        assert_eq!(n.domain, "proj");
        assert_eq!(n.instance, DEFAULT_INSTANCE);
    }
    assert!(rx.try_recv().is_err(), "不应有多余通知");

    // 日志逐行 +1：DomainLoaded..Terminated 共 8 行
    let lines = log_lines(dir.path(), "proj");
    assert_eq!(lines.len(), 8);
    for (i, line) in lines.iter().enumerate() {
        assert!(
            line.contains(&format!("\"seq\":{}", i + 1)),
            "第 {} 行 seq 应为 {}",
            i + 1,
            i + 1
        );
    }
    assert!(lines.last().unwrap().contains("\"kind\":\"Terminated\""));

    engine.shutdown().await.unwrap();
}

// ============ 2. 重放等价（快照路径 与 纯重放路径） ============

#[tokio::test]
async fn replay_equivalence_snapshot_and_pure_replay() {
    let dir = tempfile::tempdir().unwrap();

    // 第一次运行：快照路径恢复（shutdown 写终态快照）
    let (counter, archive, status) = {
        let engine = setup(dir.path()).await;
        engine
            .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
            .await
            .unwrap();
        run_partial_flow(&engine, "proj").await;
        (
            engine.counter(key("proj"), None).await.unwrap(),
            engine.archive(key("proj"), None).await.unwrap(),
            engine.status(key("proj")).await.unwrap(),
        )
    };
    {
        let engine2 = setup(dir.path()).await; // 恢复含快照快速路径
        assert_eq!(engine2.counter(key("proj"), None).await.unwrap(), counter);
        assert_eq!(engine2.archive(key("proj"), None).await.unwrap(), archive);
        assert_eq!(engine2.status(key("proj")).await.unwrap(), status);
        let listings = engine2.list_domains().await.unwrap();
        assert_eq!(listings.len(), 1);
        assert_eq!(listings[0].name, "proj");
        assert_eq!(listings[0].instances, vec![DEFAULT_INSTANCE]);
        engine2.shutdown().await.unwrap(); // 确保句柄生命周期结束
    }

    // 第二次运行：删除快照 → 纯重放路径恢复，状态仍逐字段等价
    std::fs::remove_file(snapshot_path(dir.path(), "proj")).unwrap();
    {
        let engine3 = setup(dir.path()).await;
        assert_eq!(engine3.counter(key("proj"), None).await.unwrap(), counter);
        assert_eq!(engine3.archive(key("proj"), None).await.unwrap(), archive);
        assert_eq!(engine3.status(key("proj")).await.unwrap(), status);
        // 终止状态也应重导出（counter 非空 → 未终止）
        assert!(!status.terminated);
        engine3.shutdown().await.unwrap();
    }
}

// ============ 3. 崩溃恢复：尾部半行截断 ============

#[tokio::test]
async fn crash_recovery_truncates_partial_tail() {
    let dir = tempfile::tempdir().unwrap();

    // 崩溃前：完成 seed + produce（不 shutdown，无终态快照）
    let engine = setup(dir.path()).await;
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    let design_id = run_partial_flow(&engine, "proj").await;
    crash(engine).await;

    // 模拟崩溃残留：日志尾部写入半行 JSON（无换行）
    {
        use std::io::Write;
        let path = log_path(dir.path(), "proj");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(br#"{"kind":"Consumed","seq":5,"ts":"T99""#)
            .unwrap();
    }
    assert_eq!(
        log_lines(dir.path(), "proj").len(),
        5,
        "半行占一行（无换行符）"
    );

    // 重启：截断半行，重放到 seq 4；后续命令 seq 续接
    let engine2 = setup(dir.path()).await;
    let status = engine2.status(key("proj")).await.unwrap();
    assert_eq!((status.counter_size, status.archive_size), (1, 1));
    assert_eq!(log_lines(dir.path(), "proj").len(), 4, "半行应被截断");

    // 恢复后可继续工作：消费 design token，Rule-T 触发
    engine2
        .consume(key("proj"), par("developer"), design_id, None)
        .await
        .unwrap();
    let status = engine2.status(key("proj")).await.unwrap();
    assert_eq!(
        (status.counter_size, status.archive_size, status.terminated),
        (0, 2, true)
    );
    assert_eq!(
        log_lines(dir.path(), "proj").len(),
        6,
        "Consumed(5) + Terminated(6)"
    );

    engine2.shutdown().await.unwrap();
}

// ============ 4. 崩溃恢复：索引丢失重建 ============

#[tokio::test]
async fn index_loss_triggers_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    {
        let engine = setup(dir.path()).await;
        engine
            .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
            .await
            .unwrap();
        run_partial_flow(&engine, "proj").await;
        engine.shutdown().await.unwrap();
    }

    // 索引文件丢失（真相源日志完好）
    std::fs::remove_file(dir.path().join("index.tursodb")).unwrap();

    // 顺序 id 在重启后会从 0 重来；非空实例必须换起点，避免与历史 id 冲突。
    let engine2 = setup_at(dir.path(), 100).await;
    let status = engine2.status(key("proj")).await.unwrap();
    assert_eq!((status.counter_size, status.archive_size), (1, 1));

    // 重建后引擎继续可用（索引写入路径正常）
    engine2
        .seed(
            key("proj"),
            par("external"),
            tyn("requirement"),
            ThingRef::some("urn:req:2"),
        )
        .await
        .unwrap();
    assert_eq!(engine2.counter(key("proj"), None).await.unwrap().size, 2);
    engine2.shutdown().await.unwrap();
}

// ============ 5. 崩溃恢复：中部损坏 fail-fast ============

#[tokio::test]
async fn corrupt_middle_line_fails_fast() {
    let dir = tempfile::tempdir().unwrap();
    {
        let engine = setup(dir.path()).await;
        engine
            .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
            .await
            .unwrap();
        run_partial_flow(&engine, "proj").await;
        engine.shutdown().await.unwrap();
    }

    // 中部行写坏（完整行、合法换行、非法 JSON）——不可自愈
    let path = log_path(dir.path(), "proj");
    let mut lines = log_lines(dir.path(), "proj");
    lines[1] = "{\"kind\":\"Seeded\",\"seq\":2,BROKEN".into();
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();

    let engine2 = setup(dir.path()).await;
    let err = engine2.status(key("proj")).await.unwrap_err();
    assert!(
        matches!(err, EngineError::Recovery(_)),
        "应 fail-fast：{err:?}"
    );
    // 后续命令同样以恢复失败应答
    let err = engine2.list_domains().await.unwrap_err();
    assert!(matches!(err, EngineError::Recovery(_)));
}

// ============ 6. 多域会话隔离 ============

#[tokio::test]
async fn multi_domain_sessions_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;

    engine
        .load_domain("alpha", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    engine
        .load_domain("beta", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    assert_eq!(
        domain_names(engine.list_domains().await.unwrap()),
        vec!["alpha", "beta"]
    );

    let mut rx_alpha = engine.subscribe(key("alpha")).await.unwrap();

    // 仅在 alpha 注入
    let req = engine
        .seed(
            key("alpha"),
            par("external"),
            tyn("requirement"),
            ThingRef::some("urn:req:1"),
        )
        .await
        .unwrap();

    // beta 不受影响（状态、通知均隔离）
    assert_eq!(engine.status(key("beta")).await.unwrap().counter_size, 0);
    assert_eq!(
        engine
            .counter(key("beta"), Some(tyn("requirement")))
            .await
            .unwrap()
            .size,
        0
    );

    // beta 上消费 alpha 的 token → 域不存在该 token，拒绝
    let err = engine
        .consume(key("beta"), par("architect"), req.id.clone(), None)
        .await
        .unwrap_err();
    assert!(matches!(err, EngineError::Rule(_)));

    // alpha 通知正常，且不含 beta 的任何事件
    let n = rx_alpha.try_recv().unwrap();
    assert_eq!(
        (n.event, n.domain.as_str(), n.seq),
        (NotificationKind::Seeded, "alpha", 2)
    );
    assert!(rx_alpha.try_recv().is_err());

    // 同一参与者跨域互不感知：beta 的 architect 无生产选项
    assert!(engine
        .enabled(key("beta"), par("architect"))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        engine
            .enabled(key("alpha"), par("architect"))
            .await
            .unwrap()
            .len(),
        1
    );

    // 未加载的实例报 InstanceNotFound
    let err = engine.status(key("ghost")).await.unwrap_err();
    assert!(matches!(err, EngineError::InstanceNotFound { .. }));

    engine.shutdown().await.unwrap();
}

// ============ 7. 并发命令串行化 ============

#[tokio::test]
async fn concurrent_commands_serialize() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();

    // 8 个并发 seed：单权威串行化 → 全部成功、seq 唯一递增
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let e = engine.clone();
            tokio::spawn(async move {
                e.seed(
                    key("proj"),
                    par("external"),
                    tyn("requirement"),
                    ThingRef::some(format!("urn:req:{i}")),
                )
                .await
            })
        })
        .collect();
    let mut ids = Vec::new();
    for h in handles {
        ids.push(h.await.unwrap().unwrap().id);
    }
    assert_eq!(ids.len(), 8);
    assert_eq!(
        ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
        8,
        "id 唯一"
    );

    // 日志：DomainLoaded(1) + Seeded(2..=9)，逐行 +1
    let lines = log_lines(dir.path(), "proj");
    assert_eq!(lines.len(), 9);
    for (i, line) in lines.iter().enumerate() {
        assert!(line.contains(&format!("\"seq\":{}", i + 1)));
    }

    // 重启后重放等价：8 枚 token 全部恢复
    engine.shutdown().await.unwrap();
    let engine2 = setup(dir.path()).await;
    assert_eq!(engine2.status(key("proj")).await.unwrap().counter_size, 8);
    engine2.shutdown().await.unwrap();
}

// ============ 8. 域替换重置 ============

#[tokio::test]
async fn domain_replace_resets_state_and_recovery_uses_last_load() {
    let dir = tempfile::tempdir().unwrap();

    {
        let engine = setup(dir.path()).await;
        engine
            .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
            .await
            .unwrap();
        run_partial_flow(&engine, "proj").await;
        assert_eq!(engine.status(key("proj")).await.unwrap().counter_size, 1);

        // 替换重载：状态清零、seq 续接（DomainLoaded(5)）
        let summary = engine
            .load_domain("proj", DECL, DEFAULT_INSTANCE, true)
            .await
            .unwrap();
        assert_eq!(summary.last_seq, 5);
        let status = engine.status(key("proj")).await.unwrap();
        assert_eq!((status.counter_size, status.archive_size), (0, 0));
        assert_eq!(log_lines(dir.path(), "proj").len(), 5);

        // 替换后注入新 token（Seeded(6)）
        engine
            .seed(
                key("proj"),
                par("external"),
                tyn("requirement"),
                ThingRef::some("urn:req:9"),
            )
            .await
            .unwrap();
    }

    // 崩溃重启：恢复只重放最后一次 DomainLoaded(5) 之后的事件
    let engine2 = setup(dir.path()).await;
    let counter = engine2.counter(key("proj"), None).await.unwrap();
    assert_eq!(counter.size, 1, "只有替换后的 token");
    assert_eq!(counter.tokens[0].thing_ref, ThingRef::some("urn:req:9"));
    assert_eq!(
        engine2.status(key("proj")).await.unwrap().archive_size,
        0,
        "替换前的 Archive 不复活"
    );
    engine2.shutdown().await.unwrap();
}

// ============ 9. 校验/分析/组合命令 ============

#[tokio::test]
async fn validate_analyze_compose_commands() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;

    // 非法声明 → Parse 错误（不落盘）
    let err = engine.validate("types { a }").await.unwrap_err();
    assert!(matches!(err, EngineError::Parse(_)));

    // 分析：完整生命周期域无死锁、全覆盖、良构且无警告
    let report = engine.analyze(DECL).await.unwrap();
    assert!(report.gate_coverage.is_empty(), "非源类型全部有 producer");
    assert!(report.structural_deadlock.is_empty());
    assert!(report.precondition_wf.well_formed);
    assert!(report.precondition_wf.violations.is_empty());
    assert!(report.precondition_wf.warnings.is_empty());

    // 组合并加载：两个片段（requirement→design 与 design→implementation）
    let frag1 = "types { requirement; design }
        preconditions { requirement → design }
        gates { architect: consumes requirement, produces design }
        seed_authority { external: seeds requirement }";
    let frag2 = "types { design; implementation }
        preconditions { design → implementation }
        gates { developer: consumes design, produces implementation }
        seed_authority { }";
    let summary = engine
        .compose_and_load(
            "merged",
            vec![frag1.to_owned(), frag2.to_owned()],
            DEFAULT_INSTANCE,
        )
        .await
        .unwrap();
    assert_eq!(summary.types, 3);
    assert_eq!(
        domain_names(engine.list_domains().await.unwrap()),
        vec!["merged"]
    );

    // 组合域可正常运转
    let req = engine
        .seed(
            key("merged"),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    engine
        .produce(
            key("merged"),
            par("architect"),
            tyn("design"),
            vec![req.id.clone()],
            ThingRef::none(),
        )
        .await
        .unwrap();
    assert_eq!(engine.status(key("merged")).await.unwrap().counter_size, 2);

    engine.shutdown().await.unwrap();
}

// ============ 10. 确定性：时间与 id 注入 ============

#[tokio::test]
async fn logical_clock_and_sequential_ids_are_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    let t = engine
        .seed(
            key("proj"),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    // LogicalClock：T0（DomainLoaded 占用）→ seed 为 T1；SequentialIdGen：seq-0
    assert_eq!(t.produced_at, "T1");
    assert_eq!(t.id, TokenId("seq-0".into()));
    engine.shutdown().await.unwrap();
}

// ============ 11. Rule-E 幂等（request_id） ============

#[tokio::test]
async fn consume_request_id_is_idempotent_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    let req = engine
        .seed(
            key("proj"),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();

    // 首次消费：成功归档
    let first = engine
        .consume(
            key("proj"),
            par("architect"),
            req.id.clone(),
            Some("req-1".into()),
        )
        .await
        .unwrap();
    assert_eq!(first.token.id, req.id);

    // 重试（同 request_id、同 token）：返回首次结果，不重复消费、不再落盘
    let lines_before = log_lines(dir.path(), "proj").len();
    let retry = engine
        .consume(
            key("proj"),
            par("architect"),
            req.id.clone(),
            Some("req-1".into()),
        )
        .await
        .unwrap();
    assert_eq!(retry, first, "重试返回首次归档结果");
    assert_eq!(
        log_lines(dir.path(), "proj").len(),
        lines_before,
        "重试不产生新事件"
    );
    let status = engine.status(key("proj")).await.unwrap();
    assert_eq!((status.counter_size, status.archive_size), (0, 1));

    // 同一 request_id 用于另一 token → 幂等冲突
    let req2 = engine
        .seed(
            key("proj"),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    let err = engine
        .consume(
            key("proj"),
            par("architect"),
            req2.id.clone(),
            Some("req-1".into()),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, EngineError::IdempotencyConflict { .. }));

    // 跨重启：幂等表由日志重建，重试仍返回首次结果
    engine.shutdown().await.unwrap();
    let engine2 = setup(dir.path()).await;
    let retry2 = engine2
        .consume(
            key("proj"),
            par("architect"),
            req.id.clone(),
            Some("req-1".into()),
        )
        .await
        .unwrap();
    assert_eq!(retry2, first, "重启后幂等表仍生效");

    // 不带 request_id 的正常消费不受幂等表影响
    let fresh = engine2
        .consume(key("proj"), par("architect"), req2.id.clone(), None)
        .await
        .unwrap();
    assert_eq!(fresh.token.id, req2.id);
    engine2.shutdown().await.unwrap();
}

// ============ 13. 日志前缀压缩与 schema 版本化（D25 子项 1） ============

/// 带配置的引擎（压缩测试用）。
async fn setup_with(dir: &std::path::Path, config: EngineConfig) -> Engine {
    let store = Store::open(dir).await.unwrap();
    Engine::spawn_with_config(
        store,
        Arc::new(LogicalClock::new()),
        Arc::new(SequentialIdGen::new()),
        config,
    )
}

#[tokio::test]
async fn compact_truncates_prefix_preserving_state_and_idempotency() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    let req = engine
        .seed(
            key("proj"),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    let first = engine
        .consume(
            key("proj"),
            par("architect"),
            req.id.clone(),
            Some("req-1".into()),
        )
        .await
        .unwrap();

    // 压缩前 4 行（DomainLoaded/Seeded/Consumed/Terminated）；
    // 压缩（keep_last = 0）后日志清空、快照承载全部状态
    assert_eq!(log_lines(dir.path(), "proj").len(), 4);
    let out = engine.compact(key("proj"), 0).await.unwrap();
    assert_eq!((out.snapshot_seq, out.base_seq), (4, 4));
    assert_eq!((out.retained_events, out.discarded_events), (0, 4));
    assert!(log_lines(dir.path(), "proj").is_empty());

    // 压缩不改状态；seq 自 5 续接（不重号）
    let status = engine.status(key("proj")).await.unwrap();
    assert_eq!((status.counter_size, status.archive_size), (0, 1));
    assert!(
        status.terminated,
        "消费唯一 token 后 Counter 空 → Rule-T 成立"
    );
    let req2 = engine
        .seed(
            key("proj"),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    assert_eq!(
        log_lines(dir.path(), "proj").len(),
        1,
        "新事件自 seq=5 追加"
    );
    assert!(log_lines(dir.path(), "proj")[0].contains(r#""seq":5"#));

    // 重启：日志已无 DomainLoaded，靠快照锚恢复（状态 + 幂等表）
    engine.shutdown().await.unwrap();
    let engine2 = setup(dir.path()).await;
    let status2 = engine2.status(key("proj")).await.unwrap();
    assert_eq!((status2.counter_size, status2.archive_size), (1, 1));
    assert_eq!(log_lines(dir.path(), "proj").len(), 1);
    assert!(log_lines(dir.path(), "proj")[0].contains(r#""seq":5"#));
    let retry = engine2
        .consume(
            key("proj"),
            par("architect"),
            req.id.clone(),
            Some("req-1".into()),
        )
        .await
        .unwrap();
    assert_eq!(retry, first, "压缩+重启后幂等表仍生效（快照承载）");
    let _ = req2;
    engine2.shutdown().await.unwrap();
}

#[tokio::test]
async fn compact_keep_last_retains_tail_and_schema_version_on_wire() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    let mut rx = engine.subscribe(key("proj")).await.unwrap();
    let design = run_partial_flow(&engine, "proj").await;

    // 通知顶层携带 schema 版本
    let note = rx.recv().await.unwrap();
    assert_eq!(note.version, pact_store::EVENT_SCHEMA_VERSION);

    // keep_last = 1：保留末尾 1 行（Consumed seq=4）
    let out = engine.compact(key("proj"), 1).await.unwrap();
    assert_eq!(
        (out.base_seq, out.retained_events, out.discarded_events),
        (3, 1, 3)
    );
    let lines = log_lines(dir.path(), "proj");
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains(r#""version":1"#), "事件行含 schema 版本");
    assert!(lines[0].contains(r#""seq":4"#));

    // 尾部继续产生新事件
    let impl_t = engine
        .produce(
            key("proj"),
            par("developer"),
            tyn("implementation"),
            vec![design.clone()],
            ThingRef::none(),
        )
        .await
        .unwrap();
    let lines = log_lines(dir.path(), "proj");
    assert_eq!(lines.len(), 2);
    assert!(lines[1].contains(r#""seq":5"#), "续接 seq 正确");
    let _ = impl_t;

    engine.shutdown().await.unwrap();
    let engine2 = setup(dir.path()).await;
    let status = engine2.status(key("proj")).await.unwrap();
    assert_eq!(status.archive_size, 1);
    let lines = log_lines(dir.path(), "proj");
    assert_eq!(lines.len(), 2);
    assert!(lines[1].contains(r#""seq":5"#), "重启后 seq 续接正确");
    engine2.shutdown().await.unwrap();
}

#[tokio::test]
async fn compact_on_start_config_compacts_after_recovery() {
    let dir = tempfile::tempdir().unwrap();
    {
        let engine = setup(dir.path()).await;
        engine
            .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
            .await
            .unwrap();
        run_partial_flow(&engine, "proj").await;
        engine.shutdown().await.unwrap();
    }
    assert_eq!(log_lines(dir.path(), "proj").len(), 4);

    // 启动时自动压缩（keep_last = 0）
    let engine = setup_with(
        dir.path(),
        EngineConfig {
            compact_on_start: true,
            compact_keep_last: 0,
            ..EngineConfig::default()
        },
    )
    .await;
    // 先经一次命令串行化（确保 actor 已完成恢复与启动压缩）
    let status = engine.status(key("proj")).await.unwrap();
    assert_eq!((status.counter_size, status.archive_size), (1, 1));
    assert!(
        log_lines(dir.path(), "proj").is_empty(),
        "启动自动压缩已清空前缀"
    );
    engine.shutdown().await.unwrap();
}

// ============ 12. 域双键：同一声明并发承载多个实例（D27） ============

#[tokio::test]
async fn same_declaration_supports_multiple_instances() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;

    // default 实例 + 额外实例 proj/x
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    engine.instantiate("proj", "x").await.unwrap();

    // 清单：一个定义键、两个实例
    let listings = engine.list_domains().await.unwrap();
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].name, "proj");
    assert_eq!(listings[0].instances, vec!["default", "x"]);

    let kd = key("proj");
    let kx = InstanceKey::new("proj", "x");

    // 两实例并发注入，Counter 相互独立
    engine
        .seed(
            kd.clone(),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    engine
        .seed(
            kx.clone(),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    engine
        .seed(
            kx.clone(),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    assert_eq!(engine.counter(kd.clone(), None).await.unwrap().size, 1);
    assert_eq!(engine.counter(kx.clone(), None).await.unwrap().size, 2);

    // 日志分文件：各自 `default`/`x` 独立 seq（均从 DomainLoaded(1) 起）
    assert_eq!(log_lines(dir.path(), "proj").len(), 2);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("proj").join("x.events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        3
    );

    // 未声明域实例化 → DomainNotDeclared；重复实例化 → InstanceExists
    let err = engine.instantiate("ghost", "x").await.unwrap_err();
    assert!(matches!(err, EngineError::DomainNotDeclared { .. }));
    let err = engine.instantiate("proj", "x").await.unwrap_err();
    assert!(matches!(err, EngineError::InstanceExists { .. }));

    // 非空实例再 load 且不带 force → DomainBusy；带 force → 重置该实例
    let err = engine
        .load_domain("proj", DECL, "x", false)
        .await
        .unwrap_err();
    assert!(matches!(err, EngineError::DomainBusy { .. }));
    engine.load_domain("proj", DECL, "x", true).await.unwrap();
    assert_eq!(engine.counter(kx.clone(), None).await.unwrap().size, 0);
    // default 实例不受影响
    assert_eq!(engine.counter(kd.clone(), None).await.unwrap().size, 1);

    engine.shutdown().await.unwrap();
}

// ============ 13. 声明替换不影响运行中的实例（D27 实例-声明绑定） ============

/// 与 [`DECL`] 同域但语义不同：architect 不产出 design。
const DECL_ALT: &str = r#"
types { requirement; design }

preconditions { requirement → design }

gates {
    architect: reads requirement, consumes requirement
}

seed_authority { external: seeds requirement }
"#;

#[tokio::test]
async fn replacing_declaration_does_not_disturb_running_instance() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;

    let kd = key("proj");
    let kx = InstanceKey::new("proj", "x");

    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    engine.instantiate("proj", "x").await.unwrap();

    // 在 x 上运转到「requirement 就位、architect 可产出」
    engine
        .seed(
            kx.clone(),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    assert_eq!(
        engine
            .enabled(kx.clone(), par("architect"))
            .await
            .unwrap()
            .len(),
        1
    );

    // 用 DECL_ALT 替换重载 default（force）：default 语义变化（architect 不再可产出）
    engine
        .load_domain("proj", DECL_ALT, DEFAULT_INSTANCE, true)
        .await
        .unwrap();
    engine
        .seed(
            kd.clone(),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    assert!(
        engine
            .enabled(kd.clone(), par("architect"))
            .await
            .unwrap()
            .is_empty(),
        "default 采用新声明（无 producer）"
    );

    // x 的运行态与语义不受影响（实例声明快照自封闭）
    assert_eq!(engine.counter(kx.clone(), None).await.unwrap().size, 1);
    assert_eq!(
        engine
            .enabled(kx.clone(), par("architect"))
            .await
            .unwrap()
            .len(),
        1,
        "x 仍按自身声明快照运转"
    );

    // 重启重放：x 的声明仍由自身 DomainLoaded 承载，不被替换后的声明污染
    engine.shutdown().await.unwrap();
    let engine2 = setup(dir.path()).await;
    assert_eq!(
        engine2.enabled(kx, par("architect")).await.unwrap().len(),
        1,
        "重启后 x 语义仍自封闭"
    );
    assert!(engine2
        .enabled(kd, par("architect"))
        .await
        .unwrap()
        .is_empty());
    engine2.shutdown().await.unwrap();
}

// ============ 14. 声明回填：重启后可直接实例化新实例（D27 declaration.pact） ============

#[tokio::test]
async fn declaration_backfill_supports_new_instances_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let engine = setup(dir.path()).await;
        engine
            .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
            .await
            .unwrap();
        engine.instantiate("proj", "x").await.unwrap();
        engine
            .seed(
                InstanceKey::new("proj", "x"),
                par("external"),
                tyn("requirement"),
                ThingRef::none(),
            )
            .await
            .unwrap();
        engine.shutdown().await.unwrap();
    }

    // declare 时落盘的声明原文（R4 裁决）
    let decl_file = dir.path().join("proj").join("declaration.pact");
    assert!(decl_file.exists(), "declaration.pact 应存在");
    assert!(std::fs::read_to_string(&decl_file)
        .unwrap()
        .contains("requirement"));

    // 重启：注册表由 declaration.pact 复原 → 声明级分析可用、可实例化新实例
    let engine2 = setup(dir.path()).await;
    let report = engine2.analyze_loaded("proj").await.unwrap();
    assert!(report.structural_deadlock.is_empty());

    engine2.instantiate("proj", "y").await.unwrap();
    let ky = InstanceKey::new("proj", "y");
    assert_eq!(engine2.counter(ky.clone(), None).await.unwrap().size, 0);
    // 新实例可按声明的 seed 权与 gate 正常运转
    engine2
        .seed(
            ky.clone(),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    assert_eq!(engine2.counter(ky, None).await.unwrap().size, 1);
    // 既有实例 x 的运行态保留
    assert_eq!(
        engine2
            .counter(InstanceKey::new("proj", "x"), None)
            .await
            .unwrap()
            .size,
        1
    );

    engine2.shutdown().await.unwrap();
}

// ============ 13. 读审计（D25 子项 3） ============

/// `default` 实例的读审计流路径。
fn audit_path(dir: &std::path::Path, domain: &str) -> std::path::PathBuf {
    dir.join(domain)
        .join(format!("{DEFAULT_INSTANCE}.read-audit.jsonl"))
}

#[tokio::test]
async fn read_audit_records_rule_r_without_touching_event_log() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup_with(
        dir.path(),
        EngineConfig {
            read_audit: true,
            ..EngineConfig::default()
        },
    )
    .await;
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    let req = engine
        .seed(
            key("proj"),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    engine
        .produce(
            key("proj"),
            par("architect"),
            tyn("design"),
            vec![req.id.clone()],
            ThingRef::none(),
        )
        .await
        .unwrap();

    // 两次读取 → 两条审计记录（时间正序）
    engine
        .read(key("proj"), par("architect"), req.id.clone())
        .await
        .unwrap();
    engine
        .read(key("proj"), par("architect"), req.id.clone())
        .await
        .unwrap();

    let recs = engine.read_audit(key("proj"), 10).await.unwrap();
    assert_eq!(recs.len(), 2);
    assert_eq!(recs[0].participant, "architect");
    assert_eq!(recs[0].token_id, req.id.0);
    assert_eq!(recs[0].domain, "proj");
    assert_eq!(recs[0].instance, DEFAULT_INSTANCE);
    assert_eq!(recs[0].version, pact_store::EVENT_SCHEMA_VERSION);
    assert_eq!(engine.read_audit(key("proj"), 1).await.unwrap().len(), 1);

    // 审计不进入事件日志（仍为 DomainLoaded/Seeded/Produced 三条）
    assert_eq!(log_lines(dir.path(), "proj").len(), 3);
    engine.shutdown().await.unwrap();

    // 独立持久化：重启后审计流仍可读
    let engine2 = setup_with(
        dir.path(),
        EngineConfig {
            read_audit: true,
            ..EngineConfig::default()
        },
    )
    .await;
    assert_eq!(engine2.read_audit(key("proj"), 10).await.unwrap().len(), 2);
    engine2.shutdown().await.unwrap();
}

#[tokio::test]
async fn read_audit_failure_blocks_read_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup_with(
        dir.path(),
        EngineConfig {
            read_audit: true,
            ..EngineConfig::default()
        },
    )
    .await;
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    let req = engine
        .seed(
            key("proj"),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();

    // 制造审计写入失败：以目录占位审计文件路径
    std::fs::create_dir_all(audit_path(dir.path(), "proj")).unwrap();
    let err = engine
        .read(key("proj"), par("architect"), req.id.clone())
        .await
        .unwrap_err();
    assert!(
        matches!(err, EngineError::Store(_)),
        "fail-closed：审计失败即拒绝读取，实际 {err:?}"
    );

    // 未开启读审计时，同一阻断不影响读取（默认关闭）
    let dir2 = tempfile::tempdir().unwrap();
    let off = setup(dir2.path()).await;
    off.load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    let req2 = off
        .seed(
            key("proj"),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    std::fs::create_dir_all(audit_path(dir2.path(), "proj")).unwrap();
    off.read(key("proj"), par("architect"), req2.id)
        .await
        .unwrap();
    off.shutdown().await.unwrap();
}

// ============ 15. ack 屏障：多下游完成后消费共享 token（TC4/TC13） ============

/// 两个下游各产一枚 ack，协调者按 `count ack_*` 汇聚后才消费共享 token；
/// ack 自身仍须被消费才能终止（母本 §1.7.3「Consumption barriers」）。
const ACK_BARRIER: &str = r#"
types { shared; ack_a; ack_b }

preconditions {
    shared → ack_a;
    shared → ack_b
}

gates {
    consumer_a: reads shared, produces ack_a, consumes ack_a;
    consumer_b: reads shared, produces ack_b, consumes ack_b;
    coordinator: consumes shared when count ack_a >= 1 and count ack_b >= 1
}

seed_authority { external: seeds shared }
"#;

#[tokio::test]
async fn ack_barrier_consumes_shared_after_all_downstreams() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;
    engine
        .load_domain("barrier", ACK_BARRIER, DEFAULT_INSTANCE, false)
        .await
        .unwrap();

    let shared = engine
        .seed(
            key("barrier"),
            par("external"),
            tyn("shared"),
            ThingRef::some("urn:shared:1"),
        )
        .await
        .unwrap();

    // 下游未全部 ack 前，协调者消费被 gate 拒绝。
    let err = engine
        .consume(key("barrier"), par("coordinator"), shared.id.clone(), None)
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            EngineError::Rule(RuleError::GateConditionFailed { .. })
        ),
        "屏障未满足时应拒绝消费，实际 {err:?}"
    );

    // 两个下游各自 read 共享 token 并产 ack（read 不改状态，先读后产均可）。
    engine
        .read(key("barrier"), par("consumer_a"), shared.id.clone())
        .await
        .unwrap();
    let ack_a = engine
        .produce(
            key("barrier"),
            par("consumer_a"),
            tyn("ack_a"),
            vec![shared.id.clone()],
            ThingRef::none(),
        )
        .await
        .unwrap();
    engine
        .read(key("barrier"), par("consumer_b"), shared.id.clone())
        .await
        .unwrap();
    let ack_b = engine
        .produce(
            key("barrier"),
            par("consumer_b"),
            tyn("ack_b"),
            vec![shared.id.clone()],
            ThingRef::none(),
        )
        .await
        .unwrap();

    // 屏障满足：消费共享 token；ack 仍在 Counter，须显式清理。
    engine
        .consume(
            key("barrier"),
            par("coordinator"),
            shared.id.clone(),
            Some("rid-shared".into()),
        )
        .await
        .unwrap();
    let counter = engine.counter(key("barrier"), None).await.unwrap();
    assert_eq!(counter.size, 2, "ack 不应随共享 token 被级联删除");

    engine
        .consume(key("barrier"), par("consumer_a"), ack_a.id, None)
        .await
        .unwrap();
    assert!(!engine.status(key("barrier")).await.unwrap().terminated);
    engine
        .consume(key("barrier"), par("consumer_b"), ack_b.id, None)
        .await
        .unwrap();

    // Counter 清空后才触发 Rule-T。
    let status = engine.status(key("barrier")).await.unwrap();
    assert!(status.terminated);
    let archive = engine.archive(key("barrier"), None).await.unwrap();
    assert_eq!(archive.size, 3);
    engine.shutdown().await.unwrap();
}
