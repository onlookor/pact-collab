//! 状态交接 + 活动实例别名集成测试（设计 §8.6）。
//!
//! 覆盖：状态迁移（Counter/Archive/幂等表）、源实例冻结、别名切换与回退、
//! 崩溃恢复、类型闭包失败无副作用、目标冲突与手动冻/解冻。

use pact_core::{ParticipantName, ThingRef, TokenId, TypeName};
use pact_engine::{
    Engine, EngineError, InstanceKey, LogicalClock, SequentialIdGen, DEFAULT_INSTANCE,
};
use pact_store::Store;
use std::sync::Arc;

/// 三段生命周期域；`developer` 额外持有 `reads design`，供冻结后读测试使用。
const DECL: &str = r#"
types { requirement; design; implementation }

preconditions {
    requirement → design;
    design → implementation
}

gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: reads design, consumes design, produces implementation;
    user: consumes implementation
}

seed_authority { external: seeds requirement }
"#;

/// 换版声明：在 `DECL` 上增加 `review` 类型与一条新前置/门。
const DECL_V2: &str = r#"
types { requirement; design; implementation; review }

preconditions {
    requirement → design;
    design → implementation;
    design → review
}

gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: reads design, consumes design, produces implementation;
    reviewer: consumes design, produces review;
    user: consumes implementation
}

seed_authority { external: seeds requirement }
"#;

/// 不兼容声明：故意不声明 `requirement`（源 Counter 中仍有该类型）。
const DECL_MISSING: &str = r#"
types { design; implementation }

preconditions {
    design → implementation
}

gates {
    developer: consumes design, produces implementation
}

seed_authority { external: seeds design }
"#;

fn par(s: &str) -> ParticipantName {
    ParticipantName(s.into())
}
fn tyn(s: &str) -> TypeName {
    TypeName(s.into())
}
fn key(domain: &str, instance: &str) -> InstanceKey {
    InstanceKey::new(domain, instance)
}

async fn setup_at(dir: &std::path::Path, id_start: u64) -> Engine {
    let store = Store::open(dir).await.unwrap();
    Engine::spawn(
        store,
        Arc::new(LogicalClock::new()),
        Arc::new(SequentialIdGen::new_at(id_start)),
    )
}

async fn setup(dir: &std::path::Path) -> Engine {
    setup_at(dir, 0).await
}

/// 模拟崩溃：丢弃句柄（不写终态快照），让 actor 排干通道后退出。
async fn crash(engine: Engine) {
    drop(engine);
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
}

/// seed requirement → produce design → 以 `req-1` consume requirement。
/// 返回 design token id；事件序列 DomainLoaded(1) Seeded(2) Produced(3) Consumed(4)。
async fn prepare_source(engine: &Engine) -> TokenId {
    let k = key("proj", DEFAULT_INSTANCE);
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
        .consume(k, par("architect"), req.id.clone(), Some("req-1".into()))
        .await
        .unwrap();
    design.id
}

#[tokio::test]
async fn handoff_copies_state_and_switches_alias() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    let design_id = prepare_source(&engine).await;

    let out = engine
        .handoff("proj", DEFAULT_INSTANCE, "v2", DECL_V2, true)
        .await
        .unwrap();
    assert_eq!(out.domain, "proj");
    assert_eq!(out.source_instance, DEFAULT_INSTANCE);
    assert_eq!(out.target_instance, "v2");
    assert_eq!(out.source_seq, 4);
    assert_eq!(out.target_seq, 1);
    assert_eq!((out.counter, out.archive, out.consumed_requests), (1, 1, 1));
    assert!(out.activated);
    assert_eq!(out.active_instance, "v2");

    // 别名已切换，短路由解析到 v2。
    assert_eq!(engine.active_instance("proj").await.unwrap(), "v2");
    let listings = engine.list_domains().await.unwrap();
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].instances, vec!["default", "v2"]);
    assert_eq!(listings[0].active.as_deref(), Some("v2"));

    // 目标继承 Counter 与 Archive。
    let target = key("proj", "v2");
    let counter = engine.counter(target.clone(), None).await.unwrap();
    assert_eq!(counter.size, 1);
    assert_eq!(counter.tokens[0].id, design_id);
    let archive = engine.archive(target.clone(), None).await.unwrap();
    assert_eq!(archive.size, 1);
    assert_eq!(archive.entries[0].token.ty, tyn("requirement"));
    assert_eq!(archive.entries[0].exhausted_by, par("architect"));

    // 幂等表随交接迁移：同一 request_id + 同一 token 返回首次归档结果。
    let requirement_id = archive.entries[0].token.id.clone();
    let replay = engine
        .consume(
            target.clone(),
            par("architect"),
            requirement_id,
            Some("req-1".into()),
        )
        .await
        .unwrap();
    assert_eq!(replay, archive.entries[0]);

    // 新声明生效：v2 可按 design → review 生产。
    let review = engine
        .produce(
            target,
            par("reviewer"),
            tyn("review"),
            vec![design_id],
            ThingRef::some("urn:review:1"),
        )
        .await
        .unwrap();
    assert_eq!(review.ty, tyn("review"));

    engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn frozen_source_is_read_only_but_replays_idempotently() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    let design_id = prepare_source(&engine).await;
    engine
        .handoff("proj", DEFAULT_INSTANCE, "v2", DECL_V2, true)
        .await
        .unwrap();

    let source = key("proj", DEFAULT_INSTANCE);

    // 写动作全部拒绝。
    assert!(matches!(
        engine
            .seed(
                source.clone(),
                par("external"),
                tyn("requirement"),
                ThingRef::none()
            )
            .await,
        Err(EngineError::InstanceFrozen { .. })
    ));
    assert!(matches!(
        engine
            .produce(
                source.clone(),
                par("architect"),
                tyn("design"),
                vec![design_id.clone()],
                ThingRef::none()
            )
            .await,
        Err(EngineError::InstanceFrozen { .. })
    ));
    assert!(matches!(
        engine
            .consume(source.clone(), par("developer"), design_id.clone(), None)
            .await,
        Err(EngineError::InstanceFrozen { .. })
    ));

    // 读取仍可用。
    let read = engine
        .read(source.clone(), par("developer"), design_id.clone())
        .await
        .unwrap();
    assert_eq!(read.id, design_id);
    assert_eq!(engine.counter(source.clone(), None).await.unwrap().size, 1);
    assert_eq!(engine.archive(source.clone(), None).await.unwrap().size, 1);

    // 幂等重放不产生状态变更，冻结时仍返回首次结果。
    let requirement_id = engine.archive(source.clone(), None).await.unwrap().entries[0]
        .token
        .id
        .clone();
    let replay = engine
        .consume(
            source,
            par("architect"),
            requirement_id,
            Some("req-1".into()),
        )
        .await
        .unwrap();
    assert_eq!(replay.token.ty, tyn("requirement"));

    engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn restart_restores_imported_state_freeze_and_alias() {
    let dir = tempfile::tempdir().unwrap();
    let design_id = {
        let engine = setup(dir.path()).await;
        engine
            .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
            .await
            .unwrap();
        let id = prepare_source(&engine).await;
        engine
            .handoff("proj", DEFAULT_INSTANCE, "v2", DECL_V2, true)
            .await
            .unwrap();
        crash(engine).await;
        id
    };

    let engine = setup_at(dir.path(), 10).await;
    assert_eq!(engine.active_instance("proj").await.unwrap(), "v2");
    let listings = engine.list_domains().await.unwrap();
    assert_eq!(listings[0].instances, vec!["default", "v2"]);
    assert_eq!(listings[0].active.as_deref(), Some("v2"));

    // 源仍冻结。
    assert!(matches!(
        engine
            .seed(
                key("proj", DEFAULT_INSTANCE),
                par("external"),
                tyn("requirement"),
                ThingRef::none()
            )
            .await,
        Err(EngineError::InstanceFrozen { .. })
    ));

    // 目标从 `Imported` 恢复。
    let target = key("proj", "v2");
    let counter = engine.counter(target.clone(), None).await.unwrap();
    assert_eq!(counter.size, 1);
    assert_eq!(counter.tokens[0].id, design_id);
    assert_eq!(engine.archive(target, None).await.unwrap().size, 1);

    engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn incompatible_type_closure_has_no_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    engine
        .seed(
            key("proj", DEFAULT_INSTANCE),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();

    let err = engine
        .handoff("proj", DEFAULT_INSTANCE, "v2", DECL_MISSING, true)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        EngineError::HandoffIncompatible { ref detail } if detail.contains("requirement")
    ));

    // 源未冻结：仍可继续注入种子。
    engine
        .seed(
            key("proj", DEFAULT_INSTANCE),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();

    // 目标未创建、别名不变。
    assert!(matches!(
        engine.counter(key("proj", "v2"), None).await,
        Err(EngineError::InstanceNotFound { .. })
    ));
    let listings = engine.list_domains().await.unwrap();
    assert_eq!(listings[0].instances, vec![DEFAULT_INSTANCE]);
    assert_eq!(listings[0].active, None);

    engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn target_exists_and_activate_missing_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    prepare_source(&engine).await;
    engine
        .handoff("proj", DEFAULT_INSTANCE, "v2", DECL_V2, true)
        .await
        .unwrap();

    assert!(matches!(
        engine
            .handoff("proj", DEFAULT_INSTANCE, "v2", DECL_V2, true)
            .await,
        Err(EngineError::InstanceExists { .. })
    ));
    assert!(matches!(
        engine.activate("proj", "nope").await,
        Err(EngineError::InstanceNotFound { .. })
    ));

    engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn alias_defaults_to_default_and_unload_clears_pointer() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();

    // 无指针 → 回退 default。
    assert_eq!(
        engine.active_instance("proj").await.unwrap(),
        DEFAULT_INSTANCE
    );
    assert_eq!(engine.list_domains().await.unwrap()[0].active, None);

    prepare_source(&engine).await;
    engine
        .handoff("proj", DEFAULT_INSTANCE, "v2", DECL_V2, true)
        .await
        .unwrap();
    assert_eq!(engine.active_instance("proj").await.unwrap(), "v2");

    // 卸载活动实例 → 清除指针，短路由回退 default。
    engine.unload_domain(key("proj", "v2")).await.unwrap();
    assert_eq!(
        engine.active_instance("proj").await.unwrap(),
        DEFAULT_INSTANCE
    );
    assert_eq!(engine.list_domains().await.unwrap()[0].active, None);

    engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn manual_freeze_and_unfreeze_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    engine
        .set_frozen(key("proj", DEFAULT_INSTANCE), true)
        .await
        .unwrap();
    assert!(matches!(
        engine
            .seed(
                key("proj", DEFAULT_INSTANCE),
                par("external"),
                tyn("requirement"),
                ThingRef::none()
            )
            .await,
        Err(EngineError::InstanceFrozen { .. })
    ));
    crash(engine).await;

    // 冻结标记持久化：重启后仍只读。
    let engine = setup_at(dir.path(), 10).await;
    assert!(matches!(
        engine
            .seed(
                key("proj", DEFAULT_INSTANCE),
                par("external"),
                tyn("requirement"),
                ThingRef::none()
            )
            .await,
        Err(EngineError::InstanceFrozen { .. })
    ));
    engine
        .set_frozen(key("proj", DEFAULT_INSTANCE), false)
        .await
        .unwrap();
    crash(engine).await;

    // 解冻后恢复可写。
    let engine = setup_at(dir.path(), 20).await;
    engine
        .seed(
            key("proj", DEFAULT_INSTANCE),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();

    engine.shutdown().await.unwrap();
}

/// 回归：卸载后的冻结源不能借实例化/重载绕过冻结；只有显式 force 才解除。
#[tokio::test]
async fn frozen_source_rejects_reinstantiation_without_force() {
    let dir = tempfile::tempdir().unwrap();
    {
        let engine = setup(dir.path()).await;
        engine
            .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
            .await
            .unwrap();
        prepare_source(&engine).await;
        engine
            .handoff("proj", DEFAULT_INSTANCE, "v2", DECL_V2, true)
            .await
            .unwrap();
        engine
            .unload_domain(key("proj", DEFAULT_INSTANCE))
            .await
            .unwrap();

        // 卸载后重新实例化与重载都被冻结标记拒绝。
        assert!(matches!(
            engine.instantiate("proj", DEFAULT_INSTANCE).await,
            Err(EngineError::InstanceFrozen { .. })
        ));
        assert!(matches!(
            engine
                .load_domain("proj", DECL_V2, DEFAULT_INSTANCE, false)
                .await,
            Err(EngineError::InstanceFrozen { .. })
        ));
        crash(engine).await;
    }

    // 重启后源仍冻结，且状态未被重置。
    let engine = setup_at(dir.path(), 10).await;
    assert!(matches!(
        engine
            .seed(
                key("proj", DEFAULT_INSTANCE),
                par("external"),
                tyn("requirement"),
                ThingRef::none()
            )
            .await,
        Err(EngineError::InstanceFrozen { .. })
    ));
    assert_eq!(
        engine
            .counter(key("proj", DEFAULT_INSTANCE), None)
            .await
            .unwrap()
            .size,
        1
    );

    // 显式 force 重载才解除冻结（重置语义）。
    engine
        .load_domain("proj", DECL_V2, DEFAULT_INSTANCE, true)
        .await
        .unwrap();
    engine
        .seed(
            key("proj", DEFAULT_INSTANCE),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    crash(engine).await;

    // 解冻状态持久化：重启后仍可写。
    let engine = setup_at(dir.path(), 20).await;
    engine
        .seed(
            key("proj", DEFAULT_INSTANCE),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();

    engine.shutdown().await.unwrap();
}

/// 回归：目标实例残留 `.sealed` 时交接必须拒绝，不能静默覆盖。
#[tokio::test]
async fn handoff_rejects_target_with_stale_sealed_marker() {
    let dir = tempfile::tempdir().unwrap();
    let engine = setup(dir.path()).await;
    engine
        .load_domain("proj", DECL, DEFAULT_INSTANCE, false)
        .await
        .unwrap();
    prepare_source(&engine).await;

    std::fs::write(dir.path().join("proj").join("orphan.sealed"), "stale").unwrap();
    assert!(matches!(
        engine
            .handoff("proj", DEFAULT_INSTANCE, "orphan", DECL_V2, true)
            .await,
        Err(EngineError::InstanceExists { .. })
    ));

    // 失败无副作用：源可写、目标未创建、别名未切换。
    engine
        .seed(
            key("proj", DEFAULT_INSTANCE),
            par("external"),
            tyn("requirement"),
            ThingRef::none(),
        )
        .await
        .unwrap();
    assert!(matches!(
        engine.counter(key("proj", "orphan"), None).await,
        Err(EngineError::InstanceNotFound { .. })
    ));
    assert_eq!(
        engine.active_instance("proj").await.unwrap(),
        DEFAULT_INSTANCE
    );

    engine.shutdown().await.unwrap();
}
