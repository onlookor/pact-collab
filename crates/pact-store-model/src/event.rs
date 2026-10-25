//! 事件模型（真相源，设计 §7.1；实现偏差见 docs/design-change.md：
//! DomainLoaded 也携带 seq/ts，使"逐行 +1 单调"不变量对所有行统一）。

use crate::error::ModelError;
use pact_core::{CounterState, Effect, ParticipantName, Timestamp, Token, TokenId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 事件 schema 默认版本（日志行与线上事件共用，D25 子项 1）。
///
/// 兼容契约（设计 §7.5）：
/// - 普通事件写 `EVENT_SCHEMA_VERSION`；`Imported` 写 v2（`Imported` 携带状态基线，
///   旧读者不可按 v1 语义忽略它）；
/// - 缺失 `version` 字段的历史日志行按 **v1** 解析（`serde` 缺省）；
/// - 版本高于 `SUPPORTED_EVENT_SCHEMA_VERSION` → `ModelError::UnsupportedSchemaVersion`
///   （fail-fast，不静默降级，避免把未知语义当 v1 重放）。
pub const EVENT_SCHEMA_VERSION: u32 = 1;

/// `Imported` 事件引入的 schema 版本（状态交接基线）。
pub const IMPORTED_EVENT_SCHEMA_VERSION: u32 = 2;

/// 本实现可解析的最高 schema 版本。
pub const SUPPORTED_EVENT_SCHEMA_VERSION: u32 = IMPORTED_EVENT_SCHEMA_VERSION;

fn default_schema_version() -> u32 {
    EVENT_SCHEMA_VERSION
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase")]
pub enum Event {
    /// 域加载/替换：重置会话状态（声明解析由引擎完成，store 不依赖 parser）。
    DomainLoaded {
        seq: u64,
        ts: Timestamp,
        name: String,
        declaration: String,
    },
    Seeded {
        seq: u64,
        ts: Timestamp,
        token: Token,
    },
    Produced {
        seq: u64,
        ts: Timestamp,
        token: Token,
    },
    /// 消费：token 移出 Counter 入 Archive，携带 token 全量快照，使单行日志自足
    /// （审计与重放均不依赖同一域实例其他行；`exhausted_at` = `ts`，`exhausted_by` = `by`）。
    /// `request_id` 为可选幂等键（客户端重试去重；见引擎提交契约）。
    Consumed {
        seq: u64,
        ts: Timestamp,
        token: Token,
        by: ParticipantName,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
    },
    /// Rule-T 检测结果（信息性，无状态效果，可从状态重导出）。
    Terminated { seq: u64, ts: Timestamp },
    /// 状态交接基线：新实例以新声明启动，同时继承源实例的 Counter/Archive/
    /// 幂等表。该行**自足**，携带完整状态与源日志定位（`source_seq`），
    /// 使新实例的非空状态在日志中可解释。
    Imported {
        seq: u64,
        ts: Timestamp,
        name: String,
        declaration: String,
        source_domain: String,
        source_instance: String,
        /// 导出时源实例的最后事件序号（用于定位源日志）。
        source_seq: u64,
        state: CounterState,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        consumed_requests: BTreeMap<String, (TokenId, pact_core::ArchiveEntry)>,
    },
}

impl Event {
    /// 事件的域内序号（写入方赋值；日志逐行 +1）。
    pub fn seq(&self) -> u64 {
        match self {
            Event::DomainLoaded { seq, .. }
            | Event::Seeded { seq, .. }
            | Event::Produced { seq, .. }
            | Event::Consumed { seq, .. }
            | Event::Terminated { seq, .. }
            | Event::Imported { seq, .. } => *seq,
        }
    }

    /// 该事件应写入的 schema 版本。
    pub fn schema_version(&self) -> u32 {
        match self {
            Event::Imported { .. } => IMPORTED_EVENT_SCHEMA_VERSION,
            _ => EVENT_SCHEMA_VERSION,
        }
    }

    /// 事件时间戳（ISO-8601）。
    pub fn ts(&self) -> &str {
        match self {
            Event::DomainLoaded { ts, .. }
            | Event::Seeded { ts, .. }
            | Event::Produced { ts, .. }
            | Event::Consumed { ts, .. }
            | Event::Terminated { ts, .. }
            | Event::Imported { ts, .. } => ts,
        }
    }

    /// 事件种类名（线上/索引 `kind` 列，与 §6.4 通知事件名一致）。
    pub fn kind(&self) -> &'static str {
        match self {
            Event::DomainLoaded { .. } => "DomainLoaded",
            Event::Seeded { .. } => "Seeded",
            Event::Produced { .. } => "Produced",
            Event::Consumed { .. } => "Consumed",
            Event::Terminated { .. } => "Terminated",
            Event::Imported { .. } => "Imported",
        }
    }

    /// 状态变更事件关联的 token id（无则 None）。
    pub fn token_id(&self) -> Option<&TokenId> {
        match self {
            Event::Seeded { token, .. } | Event::Produced { token, .. } => Some(&token.id),
            Event::Consumed { token, .. } => Some(&token.id),
            _ => None,
        }
    }

    /// 是否为实例状态基线（`DomainLoaded` 重置基线 / `Imported` 交接基线）。
    pub fn is_baseline(&self) -> bool {
        matches!(self, Event::DomainLoaded { .. } | Event::Imported { .. })
    }

    /// 基线事件携带的声明原文。
    pub fn baseline_declaration(&self) -> Option<(&str, &str)> {
        match self {
            Event::DomainLoaded {
                name, declaration, ..
            }
            | Event::Imported {
                name, declaration, ..
            } => Some((name, declaration)),
            _ => None,
        }
    }

    /// 覆写域内序号（构造时由引擎/store 统一赋 seq）。
    pub fn with_seq(mut self, seq: u64) -> Self {
        match &mut self {
            Event::DomainLoaded { seq: s, .. }
            | Event::Seeded { seq: s, .. }
            | Event::Produced { seq: s, .. }
            | Event::Consumed { seq: s, .. }
            | Event::Terminated { seq: s, .. }
            | Event::Imported { seq: s, .. } => *s = seq,
        }
        self
    }
}

/// 日志行 / 线上事件的封装：顶层 `version` + 事件体（内部 tag `kind`）。
///
/// 线上形态：`{"version":1,"kind":"Produced","seq":3,"ts":"…","token":{…}}`。
/// 事件体展开为同一 JSON 对象（`flatten`），使 `version` 与 `kind` 处于同一层。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireEvent {
    #[serde(default = "default_schema_version")]
    pub version: u32,
    #[serde(flatten)]
    pub event: Event,
}

impl WireEvent {
    /// 按事件自身的 schema 版本封装。
    pub fn current(event: Event) -> Self {
        Self {
            version: event.schema_version(),
            event,
        }
    }

    /// 解封装并校验版本：高于支持上限 → `UnsupportedSchemaVersion`（fail-fast）。
    pub fn into_event(self) -> Result<Event, ModelError> {
        if self.version > SUPPORTED_EVENT_SCHEMA_VERSION {
            return Err(ModelError::UnsupportedSchemaVersion {
                found: self.version,
                supported: SUPPORTED_EVENT_SCHEMA_VERSION,
            });
        }
        Ok(self.event)
    }
}

/// 事件 → 日志行 JSON（含顶层 `version`）。
pub fn to_wire_json(ev: &Event) -> Result<String, ModelError> {
    serde_json::to_string(&WireEvent::current(ev.clone())).map_err(|e| ModelError::Json {
        context: "事件序列化",
        source: e,
    })
}

/// `apply_event` 的结果分类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// 状态事件已应用（Seeded/Produced/Consumed）。
    Applied,
    /// 域声明事件：调用方应解析声明并重置状态。
    DomainReset { name: String, declaration: String },
    /// 交接基线：状态已被事件携带的状态整体替换，声明由调用方解析。
    Imported { name: String, declaration: String },
    /// 信息性事件（Terminated），无状态效果。
    Informational,
}

/// 把单个事件应用到 Counter 状态（重放路径，与在线路径共用 `Effect`/`apply`，
/// 保证重放与在线行为一致）。
///
/// Consumed 事件自带 token 全量快照：ArchiveEntry 不再从当前 Counter 反查，
/// 因此单行日志即可重放（含压缩后只剩尾部事件的情形）。
pub fn apply_event(state: &mut CounterState, ev: &Event) -> Result<ApplyOutcome, ModelError> {
    match ev {
        Event::DomainLoaded {
            name, declaration, ..
        } => Ok(ApplyOutcome::DomainReset {
            name: name.clone(),
            declaration: declaration.clone(),
        }),
        Event::Imported {
            name,
            declaration,
            state: imported_state,
            ..
        } => {
            // 交接基线自足：整体替换状态，而不是叠加到当前状态上。
            *state = imported_state.clone();
            Ok(ApplyOutcome::Imported {
                name: name.clone(),
                declaration: declaration.clone(),
            })
        }
        Event::Seeded { token, .. } => {
            state.apply(Effect::Seeded(token.clone()));
            Ok(ApplyOutcome::Applied)
        }
        Event::Produced { token, .. } => {
            state.apply(Effect::Produced(token.clone()));
            Ok(ApplyOutcome::Applied)
        }
        Event::Consumed { token, by, ts, .. } => {
            // 目标必须仍在 Counter 上：缺失意味着日志与状态不一致（重复消费、
            // 宿主违反同一实例 `prepare → commit` 串行契约等）。此处 fail-fast，
            // 避免同一 token 重复入 Archive、破坏审计与 TokenId 守卫。
            if !state.counter.contains_key(&token.id) {
                return Err(ModelError::ConsumeTargetNotOnCounter {
                    token_id: token.id.0.clone(),
                });
            }
            // 自足：token 全量随事件落盘，重放不再从 Counter 反查。
            state.apply(Effect::Consumed(pact_core::ArchiveEntry {
                token: token.clone(),
                exhausted_by: by.clone(),
                exhausted_at: ts.clone(),
            }));
            Ok(ApplyOutcome::Applied)
        }
        Event::Terminated { .. } => Ok(ApplyOutcome::Informational),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pact_core::{ThingRef, TypeName};

    fn token(id: &str, ty: &str) -> Token {
        Token {
            id: TokenId(id.into()),
            ty: TypeName(ty.into()),
            thing_ref: ThingRef::none(),
            produced_by: ParticipantName("p".into()),
            depends_on: vec![],
            produced_at: "T0".into(),
        }
    }

    #[test]
    fn event_json_roundtrip() {
        let ev = Event::Produced {
            seq: 3,
            ts: "T3".into(),
            token: token("t1", "design"),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""kind":"Produced""#));
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);

        let dl = Event::DomainLoaded {
            seq: 1,
            ts: "T0".into(),
            name: "d".into(),
            declaration: "types { a } ...".into(),
        };
        let json = serde_json::to_string(&dl).unwrap();
        assert!(json.contains(r#""kind":"DomainLoaded""#));
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), dl);
    }

    #[test]
    fn apply_event_consumed_is_self_contained() {
        // token 全量随事件而来：归档内容（含 exhausted_at/by）不依赖其他日志行。
        let mut state = CounterState::new();
        state.apply(Effect::Seeded(token("t1", "design")));
        let ev = Event::Consumed {
            seq: 2,
            ts: "T9".into(),
            token: token("t1", "design"),
            by: ParticipantName("dev".into()),
            request_id: None,
        };
        assert_eq!(apply_event(&mut state, &ev).unwrap(), ApplyOutcome::Applied);
        assert!(state.counter.is_empty());
        assert_eq!(state.archive.len(), 1);
        assert_eq!(state.archive[0].token.id.0, "t1");
        assert_eq!(state.archive[0].exhausted_by.0, "dev");
        assert_eq!(state.archive[0].exhausted_at, "T9");
    }

    #[test]
    fn apply_event_consumed_rejects_absent_target() {
        // 目标不在 Counter（已消费或未产生）→ fail-fast，禁止重复归档。
        let mut state = CounterState::new();
        let ev = Event::Consumed {
            seq: 2,
            ts: "T9".into(),
            token: token("t1", "design"),
            by: ParticipantName("dev".into()),
            request_id: None,
        };
        let err = apply_event(&mut state, &ev).unwrap_err();
        assert!(matches!(
            err,
            ModelError::ConsumeTargetNotOnCounter { ref token_id } if token_id == "t1"
        ));
        assert!(state.archive.is_empty());
    }

    #[test]
    fn apply_event_consumed_removes_matching_counter_token() {
        let mut state = CounterState::new();
        state.apply(Effect::Seeded(token("t1", "design")));
        let ev = Event::Consumed {
            seq: 2,
            ts: "T9".into(),
            token: token("t1", "design"),
            by: ParticipantName("dev".into()),
            request_id: None,
        };
        assert_eq!(apply_event(&mut state, &ev).unwrap(), ApplyOutcome::Applied);
        assert!(state.counter.is_empty());
        assert_eq!(state.archive.len(), 1);
    }

    #[test]
    fn imported_event_roundtrip_and_version() {
        let mut state = CounterState::new();
        state
            .counter
            .insert(TokenId("t2".into()), token("t2", "design"));
        state.archive.push(pact_core::ArchiveEntry {
            token: token("t1", "requirement"),
            exhausted_by: ParticipantName("architect".into()),
            exhausted_at: "T4".into(),
        });
        let mut consumed = BTreeMap::new();
        consumed.insert(
            "req-1".to_owned(),
            (TokenId("t1".into()), state.archive[0].clone()),
        );
        let ev = Event::Imported {
            seq: 1,
            ts: "T5".into(),
            name: "proj".into(),
            declaration: "types { requirement; design }".into(),
            source_domain: "proj".into(),
            source_instance: "default".into(),
            source_seq: 4,
            state,
            consumed_requests: consumed,
        };

        assert_eq!(ev.schema_version(), IMPORTED_EVENT_SCHEMA_VERSION);
        assert!(ev.is_baseline());
        assert_eq!(
            ev.baseline_declaration(),
            Some(("proj", "types { requirement; design }"))
        );

        let json = to_wire_json(&ev).unwrap();
        assert!(
            json.contains(r#""version":2"#),
            "Imported 应写 schema v2：{json}"
        );
        assert!(json.contains(r#""kind":"Imported""#));
        let wire: WireEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(wire.version, IMPORTED_EVENT_SCHEMA_VERSION);
        assert_eq!(wire.into_event().unwrap(), ev);
    }

    #[test]
    fn apply_event_imported_replaces_state() {
        let mut state = CounterState::new();
        state.apply(Effect::Seeded(token("old", "design")));

        let mut imported_state = CounterState::new();
        imported_state
            .counter
            .insert(TokenId("new".into()), token("new", "implementation"));
        let ev = Event::Imported {
            seq: 1,
            ts: "T1".into(),
            name: "proj".into(),
            declaration: "types { implementation }".into(),
            source_domain: "proj".into(),
            source_instance: "default".into(),
            source_seq: 3,
            state: imported_state.clone(),
            consumed_requests: BTreeMap::new(),
        };

        let outcome = apply_event(&mut state, &ev).unwrap();
        assert_eq!(
            outcome,
            ApplyOutcome::Imported {
                name: "proj".into(),
                declaration: "types { implementation }".into(),
            }
        );
        // 交接基线自足：整体替换，而不是叠加。
        assert_eq!(state, imported_state);
        assert!(!state.counter.contains_key(&TokenId("old".into())));
    }

    #[test]
    fn unsupported_schema_version_fails_fast() {
        let wire = WireEvent {
            version: SUPPORTED_EVENT_SCHEMA_VERSION + 1,
            event: Event::Terminated {
                seq: 1,
                ts: "T1".into(),
            },
        };
        let err = wire.into_event().unwrap_err();
        assert!(matches!(
            err,
            ModelError::UnsupportedSchemaVersion { found, supported }
                if found == 3 && supported == 2
        ));
    }

    #[test]
    fn consumed_event_roundtrips_request_id() {
        let ev = Event::Consumed {
            seq: 5,
            ts: "T5".into(),
            token: token("t1", "design"),
            by: ParticipantName("dev".into()),
            request_id: Some("rid-1".into()),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""request_id":"rid-1""#));
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), ev);

        // 无 request_id 时不序列化该字段（保持日志最小）
        let ev2 = Event::Consumed {
            seq: 6,
            ts: "T6".into(),
            token: token("t2", "design"),
            by: ParticipantName("dev".into()),
            request_id: None,
        };
        assert!(!serde_json::to_string(&ev2).unwrap().contains("request_id"));
    }
}
