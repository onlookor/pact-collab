//! 事件模型（真相源，设计 §7.1；实现偏差见 docs/design-change.md：
//! DomainLoaded 也携带 seq/ts，使"逐行 +1 单调"不变量对所有行统一）。

use crate::error::ModelError;
use pact_core::{CounterState, Effect, ParticipantName, Timestamp, Token, TokenId};
use serde::{Deserialize, Serialize};

/// 事件 schema 版本（日志行与线上事件共用，D25 子项 1）。
///
/// 兼容契约（设计 §7.5）：
/// - 写入方恒发当前版本（`EVENT_SCHEMA_VERSION`）；
/// - 缺失 `version` 字段的历史日志行按 **v1** 解析（`serde` 缺省）；
/// - 版本高于本实现支持上限 → `ModelError::UnsupportedSchemaVersion`（fail-fast，
///   不静默降级，避免把未知语义当 v1 重放）。
pub const EVENT_SCHEMA_VERSION: u32 = 1;

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
}

impl Event {
    /// 事件的域内序号（写入方赋值；日志逐行 +1）。
    pub fn seq(&self) -> u64 {
        match self {
            Event::DomainLoaded { seq, .. }
            | Event::Seeded { seq, .. }
            | Event::Produced { seq, .. }
            | Event::Consumed { seq, .. }
            | Event::Terminated { seq, .. } => *seq,
        }
    }

    /// 事件时间戳（ISO-8601）。
    pub fn ts(&self) -> &str {
        match self {
            Event::DomainLoaded { ts, .. }
            | Event::Seeded { ts, .. }
            | Event::Produced { ts, .. }
            | Event::Consumed { ts, .. }
            | Event::Terminated { ts, .. } => ts,
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

    /// 覆写域内序号（构造时由引擎/store 统一赋 seq）。
    pub fn with_seq(mut self, seq: u64) -> Self {
        match &mut self {
            Event::DomainLoaded { seq: s, .. }
            | Event::Seeded { seq: s, .. }
            | Event::Produced { seq: s, .. }
            | Event::Consumed { seq: s, .. }
            | Event::Terminated { seq: s, .. } => *s = seq,
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
    /// 以当前 schema 版本封装事件。
    pub fn current(event: Event) -> Self {
        Self {
            version: EVENT_SCHEMA_VERSION,
            event,
        }
    }

    /// 解封装并校验版本：高于支持上限 → `UnsupportedSchemaVersion`（fail-fast）。
    pub fn into_event(self) -> Result<Event, ModelError> {
        if self.version > EVENT_SCHEMA_VERSION {
            return Err(ModelError::UnsupportedSchemaVersion {
                found: self.version,
                supported: EVENT_SCHEMA_VERSION,
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
        Event::Seeded { token, .. } => {
            state.apply(Effect::Seeded(token.clone()));
            Ok(ApplyOutcome::Applied)
        }
        Event::Produced { token, .. } => {
            state.apply(Effect::Produced(token.clone()));
            Ok(ApplyOutcome::Applied)
        }
        Event::Consumed { token, by, ts, .. } => {
            // 自足：token 全量随事件落盘，重放不再从 Counter 反查
            // （Effect::Consumed 只按 id 移除目标（不在则无操作）并追加 Archive）。
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
        // 空 Counter 上重放 Consumed：token 全量随事件而来，无需反查同一域实例其他行
        let mut state = CounterState::new();
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
