//! pact-engine 集成测试（设计 §10 阶段 3 验收项）：
//! - 引擎命令流（多域、并发命令串行化）；
//! - 重放等价（快照快速路径 与 纯重放路径 状态一致）；
//! - 崩溃恢复（JSONL 尾部半行截断、索引丢失重建、中部损坏 fail-fast）；
//! - 订阅通知时序（事件顺序、seq 单调、变更后规模）；
//! - 域替换重置（恢复只重放最后一次 DomainLoaded 之后的事件）。

use pact_core::{ParticipantName, ThingRef, TokenId, TypeName};
use pact_engine::{Engine, EngineError, LogicalClock, NotificationKind, SequentialIdGen};
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

/// 临时数据目录 + 引擎（LogicalClock + 顺序 id，确定性）。
async fn setup(dir: &std::path::Path) -> Engine {
    let store = Store::open(dir).await.unwrap();
    Engine::spawn(
        store,
        Arc::new(LogicalClock::new()),
        Arc::new(SequentialIdGen::new()),
    )
}

/// 模拟崩溃：丢弃句柄（不写终态快照），让 actor 排干通道后退出。
async fn crash(engine: Engine) {
    drop(engine);
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
}

fn log_path(dir: &std::path::Path, domain: &str) -> std::path::PathBuf {
    dir.join(format!("{domain}.events.jsonl"))
}

fn log_lines(dir: &std::path::Path, domain: &str) -> Vec<String> {
    std::fs::read_to_string(log_path(dir, domain))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// seed → produce → consume 前进到"design 在 Counter、requirement 已归档"。
/// 事件序列：DomainLoaded(1) Seeded(2) Produced(3) Consumed(4)。
async fn run_partial_flow(engine: &Engine, domain: &str) -> TokenId {
    let req = engine
        .seed(
            domain,
            par("external"),
            tyn("requirement"),
            ThingRef::some("urn:req:1"),
        )
        .await
        .unwrap();
    let design = engine
        .produce(
            domain,
            par("architect"),
            tyn("design"),
            vec![req.id.clone()],
            ThingRef::some("urn:design:1"),
        )
        .await
        .unwrap();
    engine
        .consume(domain, par("architect"), req.id.clone(), None)
        .await
        .unwrap();
    design.id
}

// ============ 1. 命令流 + 通知时序 + Rule-W 查询 ============

#[tokio::test]
async fn full_lifecycle_with_notifications() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;

    let summary = engine.load_domain("proj", DECL).await.unwrap();
    assert_eq!((summary.types, summary.gates, summary.last_seq), (3, 6, 1));

    // Rule-W：种子注入前 architect 无可生产选项
    assert!(engine
        .enabled("proj", par("architect"))
        .await
        .unwrap()
        .is_empty());

    // 订阅先于动作（broadcast 只投递给已存在的订阅者）
    let mut rx = engine.subscribe("proj").await.unwrap();

    let req = engine
        .seed(
            "proj",
            par("external"),
            tyn("requirement"),
            ThingRef::some("urn:req:1"),
        )
        .await
        .unwrap();
    assert_eq!(req.produced_by, par("external"));

    // Rule-W：requirement 就位后 design 生产选项出现，见证前驱 = 该 token
    let enabled = engine.enabled("proj", par("architect")).await.unwrap();
    assert_eq!(enabled.len(), 1);
    assert_eq!(enabled[0].ty, tyn("design"));
    assert_eq!(enabled[0].witness_predecessors, vec![req.id.clone()]);

    // my_gates 自省
    let gates = engine.my_gates("proj", par("architect")).await.unwrap();
    assert_eq!(gates.len(), 3);
    assert!(gates.iter().any(|g| g.ty == tyn("requirement")));
    assert!(gates.iter().any(|g| g.ty == tyn("design")));

    // Rule-R 无副作用（D9）：读前后日志行数不变
    let lines_before = log_lines(dir.path(), "proj").len();
    let read_back = engine
        .read("proj", par("architect"), req.id.clone())
        .await
        .unwrap();
    assert_eq!(read_back.id, req.id);
    assert_eq!(log_lines(dir.path(), "proj").len(), lines_before);

    let design = engine
        .produce(
            "proj",
            par("architect"),
            tyn("design"),
            vec![req.id.clone()],
            ThingRef::some("urn:design:1"),
        )
        .await
        .unwrap();
    engine
        .consume("proj", par("architect"), req.id.clone(), None)
        .await
        .unwrap();
    let impl_t = engine
        .produce(
            "proj",
            par("developer"),
            tyn("implementation"),
            vec![design.id.clone()],
            ThingRef::some("urn:impl:1"),
        )
        .await
        .unwrap();
    engine
        .consume("proj", par("developer"), design.id.clone(), None)
        .await
        .unwrap();
    engine
        .consume("proj", par("user"), impl_t.id.clone(), None)
        .await
        .unwrap();

    // 终态：Counter 空、Archive 3 项、Rule-T 成立
    let status = engine.status("proj").await.unwrap();
    assert_eq!(
        (status.counter_size, status.archive_size, status.terminated),
        (0, 3, true)
    );
    assert_eq!(engine.counter("proj", None).await.unwrap().size, 0);
    assert_eq!(
        engine
            .archive("proj", Some(tyn("requirement")))
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
        engine.load_domain("proj", DECL).await.unwrap();
        run_partial_flow(&engine, "proj").await;
        (
            engine.counter("proj", None).await.unwrap(),
            engine.archive("proj", None).await.unwrap(),
            engine.status("proj").await.unwrap(),
        )
    };
    {
        let engine2 = setup(dir.path()).await; // 恢复含快照快速路径
        assert_eq!(engine2.counter("proj", None).await.unwrap(), counter);
        assert_eq!(engine2.archive("proj", None).await.unwrap(), archive);
        assert_eq!(engine2.status("proj").await.unwrap(), status);
        assert_eq!(engine2.list_domains().await.unwrap(), vec!["proj"]);
        engine2.shutdown().await.unwrap(); // 确保句柄生命周期结束
    }

    // 第二次运行：删除快照 → 纯重放路径恢复，状态仍逐字段等价
    std::fs::remove_file(dir.path().join("proj.snapshot.json")).unwrap();
    {
        let engine3 = setup(dir.path()).await;
        assert_eq!(engine3.counter("proj", None).await.unwrap(), counter);
        assert_eq!(engine3.archive("proj", None).await.unwrap(), archive);
        assert_eq!(engine3.status("proj").await.unwrap(), status);
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
    engine.load_domain("proj", DECL).await.unwrap();
    let design_id = run_partial_flow(&engine, "proj").await;
    crash(engine).await;

    // 模拟崩溃残留：日志尾部写入半行 JSON（无换行）
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(log_path(dir.path(), "proj"))
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
    let status = engine2.status("proj").await.unwrap();
    assert_eq!((status.counter_size, status.archive_size), (1, 1));
    assert_eq!(log_lines(dir.path(), "proj").len(), 4, "半行应被截断");

    // 恢复后可继续工作：消费 design token，Rule-T 触发
    engine2
        .consume("proj", par("developer"), design_id, None)
        .await
        .unwrap();
    let status = engine2.status("proj").await.unwrap();
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
        engine.load_domain("proj", DECL).await.unwrap();
        run_partial_flow(&engine, "proj").await;
        engine.shutdown().await.unwrap();
    }

    // 索引文件丢失（真相源日志完好）
    std::fs::remove_file(dir.path().join("index.tursodb")).unwrap();

    let engine2 = setup(dir.path()).await;
    let status = engine2.status("proj").await.unwrap();
    assert_eq!((status.counter_size, status.archive_size), (1, 1));

    // 重建后引擎继续可用（索引写入路径正常）
    engine2
        .seed(
            "proj",
            par("external"),
            tyn("requirement"),
            ThingRef::some("urn:req:2"),
        )
        .await
        .unwrap();
    assert_eq!(engine2.counter("proj", None).await.unwrap().size, 2);
    engine2.shutdown().await.unwrap();
}

// ============ 5. 崩溃恢复：中部损坏 fail-fast ============

#[tokio::test]
async fn corrupt_middle_line_fails_fast() {
    let dir = tempfile::tempdir().unwrap();
    {
        let engine = setup(dir.path()).await;
        engine.load_domain("proj", DECL).await.unwrap();
        run_partial_flow(&engine, "proj").await;
        engine.shutdown().await.unwrap();
    }

    // 中部行写坏（完整行、合法换行、非法 JSON）——不可自愈
    let path = log_path(dir.path(), "proj");
    let mut lines = log_lines(dir.path(), "proj");
    lines[1] = "{\"kind\":\"Seeded\",\"seq\":2,BROKEN".into();
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();

    let engine2 = setup(dir.path()).await;
    let err = engine2.status("proj").await.unwrap_err();
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

    engine.load_domain("alpha", DECL).await.unwrap();
    engine.load_domain("beta", DECL).await.unwrap();
    assert_eq!(engine.list_domains().await.unwrap(), vec!["alpha", "beta"]);

    let mut rx_alpha = engine.subscribe("alpha").await.unwrap();

    // 仅在 alpha 注入
    let req = engine
        .seed(
            "alpha",
            par("external"),
            tyn("requirement"),
            ThingRef::some("urn:req:1"),
        )
        .await
        .unwrap();

    // beta 不受影响（状态、通知均隔离）
    assert_eq!(engine.status("beta").await.unwrap().counter_size, 0);
    assert_eq!(
        engine
            .counter("beta", Some(tyn("requirement")))
            .await
            .unwrap()
            .size,
        0
    );

    // beta 上消费 alpha 的 token → 域不存在该 token，拒绝
    let err = engine
        .consume("beta", par("architect"), req.id.clone(), None)
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
        .enabled("beta", par("architect"))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        engine
            .enabled("alpha", par("architect"))
            .await
            .unwrap()
            .len(),
        1
    );

    // 未加载的域报 DomainNotFound
    let err = engine.status("ghost").await.unwrap_err();
    assert!(matches!(err, EngineError::DomainNotFound { .. }));

    engine.shutdown().await.unwrap();
}

// ============ 7. 并发命令串行化 ============

#[tokio::test]
async fn concurrent_commands_serialize() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;
    engine.load_domain("proj", DECL).await.unwrap();

    // 8 个并发 seed：单权威串行化 → 全部成功、seq 唯一递增
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let e = engine.clone();
            tokio::spawn(async move {
                e.seed(
                    "proj",
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
    assert_eq!(engine2.status("proj").await.unwrap().counter_size, 8);
    engine2.shutdown().await.unwrap();
}

// ============ 8. 域替换重置 ============

#[tokio::test]
async fn domain_replace_resets_state_and_recovery_uses_last_load() {
    let dir = tempfile::tempdir().unwrap();

    {
        let engine = setup(dir.path()).await;
        engine.load_domain("proj", DECL).await.unwrap();
        run_partial_flow(&engine, "proj").await;
        assert_eq!(engine.status("proj").await.unwrap().counter_size, 1);

        // 替换重载：状态清零、seq 续接（DomainLoaded(5)）
        let summary = engine.load_domain("proj", DECL).await.unwrap();
        assert_eq!(summary.last_seq, 5);
        let status = engine.status("proj").await.unwrap();
        assert_eq!((status.counter_size, status.archive_size), (0, 0));
        assert_eq!(log_lines(dir.path(), "proj").len(), 5);

        // 替换后注入新 token（Seeded(6)）
        engine
            .seed(
                "proj",
                par("external"),
                tyn("requirement"),
                ThingRef::some("urn:req:9"),
            )
            .await
            .unwrap();
    }

    // 崩溃重启：恢复只重放最后一次 DomainLoaded(5) 之后的事件
    let engine2 = setup(dir.path()).await;
    let counter = engine2.counter("proj", None).await.unwrap();
    assert_eq!(counter.size, 1, "只有替换后的 token");
    assert_eq!(counter.tokens[0].thing_ref, ThingRef::some("urn:req:9"));
    assert_eq!(
        engine2.status("proj").await.unwrap().archive_size,
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
        .compose_and_load("merged", vec![frag1.to_owned(), frag2.to_owned()])
        .await
        .unwrap();
    assert_eq!(summary.types, 3);
    assert_eq!(engine.list_domains().await.unwrap(), vec!["merged"]);

    // 组合域可正常运转
    let req = engine
        .seed(
            "merged",
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    engine
        .produce(
            "merged",
            par("architect"),
            tyn("design"),
            vec![req.id.clone()],
            ThingRef::none(),
        )
        .await
        .unwrap();
    assert_eq!(engine.status("merged").await.unwrap().counter_size, 2);

    engine.shutdown().await.unwrap();
}

// ============ 10. 确定性：时间与 id 注入 ============

#[tokio::test]
async fn logical_clock_and_sequential_ids_are_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;
    engine.load_domain("proj", DECL).await.unwrap();
    let t = engine
        .seed(
            "proj",
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
    engine.load_domain("proj", DECL).await.unwrap();
    let req = engine
        .seed(
            "proj",
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();

    // 首次消费：成功归档
    let first = engine
        .consume(
            "proj",
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
            "proj",
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
    let status = engine.status("proj").await.unwrap();
    assert_eq!((status.counter_size, status.archive_size), (0, 1));

    // 同一 request_id 用于另一 token → 幂等冲突
    let req2 = engine
        .seed(
            "proj",
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    let err = engine
        .consume(
            "proj",
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
            "proj",
            par("architect"),
            req.id.clone(),
            Some("req-1".into()),
        )
        .await
        .unwrap();
    assert_eq!(retry2, first, "重启后幂等表仍生效");

    // 不带 request_id 的正常消费不受幂等表影响
    let fresh = engine2
        .consume("proj", par("architect"), req2.id.clone(), None)
        .await
        .unwrap();
    assert_eq!(fresh.token.id, req2.id);
    engine2.shutdown().await.unwrap();
}
