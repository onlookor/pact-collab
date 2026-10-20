//! 派生索引（D13 隔离层）：`IndexStore` trait + turso 默认实现。
//!
//! 索引永远可丢弃重建——真相源是 JSONL 事件日志（设计 §7.4）。
//! 键为双列 `(domain, instance)`（D27 路径 β）：seq 是实例内单调，
//! 跨实例会碰撞，故 events 主键为 (domain, instance, seq)。
//! 旧单列 schema 由 `open` 自愈（缺 `instance` 列 → DROP 后按新 schema 重建）。

use crate::error::StoreError;
use crate::event::{apply_event, Event};
use crate::key::InstanceKey;
use async_trait::async_trait;
use pact_core::{CounterState, ParticipantName, ThingRef, Timestamp, Token};
use std::path::Path;
use std::sync::Arc;
use turso::{Builder, Connection, Value};

/// token 当前状态（tokens 表的 status 派生列）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenStatus {
    Counter,
    Archived {
        exhausted_by: ParticipantName,
        exhausted_at: Timestamp,
    },
}

impl TokenStatus {
    fn as_str(&self) -> &'static str {
        match self {
            TokenStatus::Counter => "counter",
            TokenStatus::Archived { .. } => "archived",
        }
    }
}

/// 派生索引抽象（D13）。默认实现 `TursoIndexStore`；
/// 阻塞性问题出现时可实现 `RusqliteIndexStore` 替换，仅动本 crate。
#[async_trait]
pub trait IndexStore: Send + Sync {
    /// 打开/创建索引（建表，幂等）。
    async fn open(path: &Path) -> Result<Self, StoreError>
    where
        Self: Sized;
    /// 写入一个事件行。
    async fn upsert_event(&self, key: &InstanceKey, ev: &Event) -> Result<(), StoreError>;
    /// 写入/更新 token 行（status = counter | archived）。
    async fn upsert_token(
        &self,
        key: &InstanceKey,
        t: &Token,
        status: &TokenStatus,
    ) -> Result<(), StoreError>;
    /// 实例内最大事件 seq（None = 该实例无事件）。启动一致性校验用。
    async fn last_event_seq(&self, key: &InstanceKey) -> Result<Option<u64>, StoreError>;
    /// 裁剪该实例中 `seq <= base_seq` 的事件行（日志前缀压缩后的索引同步）。
    async fn prune_events(&self, key: &InstanceKey, base_seq: u64) -> Result<(), StoreError>;
    /// 从全部实例日志全量重建（清空后重放）。非事务性——索引可随时重建，无需原子。
    async fn rebuild(&self, entries: &[(InstanceKey, Vec<Event>)]) -> Result<(), StoreError>;
}

fn to_index_err(e: turso::Error) -> StoreError {
    StoreError::Index(e.to_string())
}

/// turso（原 Limbo）实现。BETA 风险由"可随时重建"对冲（D13）。
pub struct TursoIndexStore {
    conn: Arc<Connection>,
}

/// 探测既有表是否已含 `instance` 列（新 schema）。
/// 表不存在或缺列 → 语句准备失败 → false（触发自愈重建）。
async fn table_has_instance(conn: &Connection, table: &str) -> bool {
    conn.query(&format!("SELECT instance FROM {table} LIMIT 0"), ())
        .await
        .is_ok()
}

#[async_trait]
impl IndexStore for TursoIndexStore {
    async fn open(path: &Path) -> Result<Self, StoreError> {
        let db = Builder::new_local(path.to_string_lossy().as_ref())
            .build()
            .await
            .map_err(to_index_err)?;
        let conn = db.connect().map_err(to_index_err)?;
        // 旧单列 schema 自愈：缺 instance 列 → DROP 后按新 schema 重建
        // （索引是纯派生物，清空后由启动一致性校验触发 rebuild，D13）。
        if !table_has_instance(&conn, "events").await || !table_has_instance(&conn, "tokens").await
        {
            conn.execute_batch("DROP TABLE IF EXISTS events; DROP TABLE IF EXISTS tokens;")
                .await
                .map_err(to_index_err)?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events(
                domain TEXT NOT NULL,
                instance TEXT NOT NULL,
                seq INTEGER NOT NULL,
                kind TEXT NOT NULL,
                ts TEXT NOT NULL,
                payload TEXT NOT NULL,
                PRIMARY KEY(domain, instance, seq));
             CREATE TABLE IF NOT EXISTS tokens(
                token_id TEXT PRIMARY KEY,
                domain TEXT NOT NULL,
                instance TEXT NOT NULL,
                type TEXT NOT NULL,
                produced_by TEXT NOT NULL,
                thing_ref TEXT,
                depends_on TEXT NOT NULL,
                produced_at TEXT NOT NULL,
                status TEXT NOT NULL,
                exhausted_by TEXT,
                exhausted_at TEXT);
             CREATE INDEX IF NOT EXISTS idx_tokens_domain_instance_type
                ON tokens(domain, instance, type, status);",
        )
        .await
        .map_err(to_index_err)?;
        Ok(Self {
            conn: Arc::new(conn),
        })
    }

    async fn upsert_event(&self, key: &InstanceKey, ev: &Event) -> Result<(), StoreError> {
        let payload = crate::event::to_wire_json(ev)?;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO events(domain, instance, seq, kind, ts, payload)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                [
                    Value::Text(key.domain.clone()),
                    Value::Text(key.instance.clone()),
                    Value::Integer(ev.seq() as i64),
                    Value::Text(ev.kind().to_owned()),
                    Value::Text(ev.ts().to_owned()),
                    Value::Text(payload),
                ],
            )
            .await
            .map_err(to_index_err)?;
        Ok(())
    }

    async fn upsert_token(
        &self,
        key: &InstanceKey,
        t: &Token,
        status: &TokenStatus,
    ) -> Result<(), StoreError> {
        let depends_on = serde_json::to_string(&t.depends_on)?;
        let thing_ref = t.thing_ref.0.clone();
        let (exhausted_by, exhausted_at) = match status {
            TokenStatus::Counter => (None, None),
            TokenStatus::Archived {
                exhausted_by,
                exhausted_at,
            } => (Some(exhausted_by.0.clone()), Some(exhausted_at.clone())),
        };
        self.conn
            .execute(
                "INSERT OR REPLACE INTO tokens(
                    token_id, domain, instance, type, produced_by, thing_ref, depends_on,
                    produced_at, status, exhausted_by, exhausted_at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                [
                    Value::Text(t.id.0.clone()),
                    Value::Text(key.domain.clone()),
                    Value::Text(key.instance.clone()),
                    Value::Text(t.ty.0.clone()),
                    Value::Text(t.produced_by.0.clone()),
                    thing_ref.map_or(Value::Null, Value::Text),
                    Value::Text(depends_on),
                    Value::Text(t.produced_at.clone()),
                    Value::Text(status.as_str().to_owned()),
                    exhausted_by.map_or(Value::Null, Value::Text),
                    exhausted_at.map_or(Value::Null, Value::Text),
                ],
            )
            .await
            .map_err(to_index_err)?;
        Ok(())
    }

    async fn last_event_seq(&self, key: &InstanceKey) -> Result<Option<u64>, StoreError> {
        let mut rows = self
            .conn
            .query(
                "SELECT MAX(seq) FROM events WHERE domain = ?1 AND instance = ?2",
                [
                    Value::Text(key.domain.clone()),
                    Value::Text(key.instance.clone()),
                ],
            )
            .await
            .map_err(to_index_err)?;
        let row = rows.next().await.map_err(to_index_err)?;
        let Some(row) = row else { return Ok(None) };
        match row.get_value(0).map_err(to_index_err)? {
            Value::Null => Ok(None),
            Value::Integer(n) => Ok(Some(n.max(0) as u64)),
            v => Err(StoreError::Index(format!("MAX(seq) 返回非整数：{v:?}"))),
        }
    }

    async fn prune_events(&self, key: &InstanceKey, base_seq: u64) -> Result<(), StoreError> {
        self.conn
            .execute(
                "DELETE FROM events WHERE domain = ?1 AND instance = ?2 AND seq <= ?3",
                [
                    Value::Text(key.domain.clone()),
                    Value::Text(key.instance.clone()),
                    Value::Integer(base_seq as i64),
                ],
            )
            .await
            .map_err(to_index_err)?;
        Ok(())
    }

    async fn rebuild(&self, entries: &[(InstanceKey, Vec<Event>)]) -> Result<(), StoreError> {
        // 清空后重放写入。非事务：索引是纯派生物，中断只会留下部分索引，
        // 下次启动的一致性校验会再次触发重建（自愈）。
        self.conn
            .execute("DELETE FROM events", ())
            .await
            .map_err(to_index_err)?;
        self.conn
            .execute("DELETE FROM tokens", ())
            .await
            .map_err(to_index_err)?;

        for (key, events) in entries {
            // 模拟重放得到 token 终态（counter | archived + 消费者/时间）
            let mut state = CounterState::new();
            for ev in events {
                apply_event(&mut state, ev)?;
                match ev {
                    Event::Seeded { token, .. } | Event::Produced { token, .. } => {
                        self.upsert_token(key, token, &TokenStatus::Counter).await?;
                    }
                    Event::Consumed { token, by, ts, .. } => {
                        // 事件自足：token 全量随行，直接投影归档终态
                        self.upsert_token(
                            key,
                            token,
                            &TokenStatus::Archived {
                                exhausted_by: by.clone(),
                                exhausted_at: ts.clone(),
                            },
                        )
                        .await?;
                    }
                    _ => {}
                }
                self.upsert_event(key, ev).await?;
            }
        }
        Ok(())
    }
}

// 未直接使用但属于 tokens 表线上语义的类型（thing_ref 可空列 ↔ ThingRef::Option）
#[allow(dead_code)]
fn _thing_ref_column(_r: &ThingRef) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Event;
    use pact_core::{TokenId, TypeName};

    fn key(domain: &str) -> InstanceKey {
        InstanceKey::new(domain, "default")
    }

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

    fn sample(domain: &str) -> (InstanceKey, Vec<Event>) {
        (
            key(domain),
            vec![
                Event::DomainLoaded {
                    seq: 1,
                    ts: "T0".into(),
                    name: domain.into(),
                    declaration: "…".into(),
                },
                Event::Seeded {
                    seq: 2,
                    ts: "T1".into(),
                    token: token(&format!("{domain}-t1"), "req"),
                },
                Event::Produced {
                    seq: 3,
                    ts: "T2".into(),
                    token: token(&format!("{domain}-t2"), "design"),
                },
                Event::Consumed {
                    seq: 4,
                    ts: "T3".into(),
                    token: token(&format!("{domain}-t1"), "req"),
                    by: ParticipantName("arch".into()),
                    request_id: None,
                },
            ],
        )
    }

    #[tokio::test]
    async fn upsert_and_last_event_seq() {
        let dir = tempfile::tempdir().unwrap();
        let idx = TursoIndexStore::open(&dir.path().join("index.tursodb"))
            .await
            .unwrap();
        let (k, events) = sample("dom");
        for ev in &events {
            idx.upsert_event(&k, ev).await.unwrap();
        }
        assert_eq!(idx.last_event_seq(&k).await.unwrap(), Some(4));
        assert_eq!(idx.last_event_seq(&key("other")).await.unwrap(), None);
    }

    #[tokio::test]
    async fn rebuild_projects_token_status() {
        let dir = tempfile::tempdir().unwrap();
        let idx = TursoIndexStore::open(&dir.path().join("index.tursodb"))
            .await
            .unwrap();
        let entries = vec![sample("dom"), sample("dom2")];
        idx.rebuild(&entries).await.unwrap();
        assert_eq!(idx.last_event_seq(&key("dom")).await.unwrap(), Some(4));
        assert_eq!(idx.last_event_seq(&key("dom2")).await.unwrap(), Some(4));

        // tokens 投影：dom 的 t1 归档、t2 在 Counter
        let mut rows = idx
            .conn
            .query(
                "SELECT status, exhausted_by FROM tokens \
                 WHERE domain = 'dom' AND instance = 'default' ORDER BY token_id",
                (),
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get_value(0).unwrap(), Value::Text("archived".into()));
        assert_eq!(row.get_value(1).unwrap(), Value::Text("arch".into()));
        let row = rows.next().await.unwrap().unwrap();
        assert_eq!(row.get_value(0).unwrap(), Value::Text("counter".into()));
        assert_eq!(row.get_value(1).unwrap(), Value::Null);
    }

    #[tokio::test]
    async fn same_domain_two_instances_keep_separate_seq() {
        let dir = tempfile::tempdir().unwrap();
        let idx = TursoIndexStore::open(&dir.path().join("index.tursodb"))
            .await
            .unwrap();
        let a = InstanceKey::new("dom", "a");
        let b = InstanceKey::new("dom", "b");
        for ev in &sample("dom").1 {
            idx.upsert_event(&a, ev).await.unwrap();
        }
        idx.upsert_event(&b, &sample("dom").1[0]).await.unwrap();
        assert_eq!(idx.last_event_seq(&a).await.unwrap(), Some(4));
        assert_eq!(idx.last_event_seq(&b).await.unwrap(), Some(1));
    }

    #[tokio::test]
    async fn legacy_schema_is_healed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.tursodb");
        // 旧单列 schema（无 instance 列）模拟
        {
            let db = Builder::new_local(path.to_string_lossy().as_ref())
                .build()
                .await
                .unwrap();
            let conn = db.connect().unwrap();
            conn.execute_batch(
                "CREATE TABLE events(
                    domain TEXT NOT NULL, seq INTEGER NOT NULL, kind TEXT NOT NULL,
                    ts TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(domain, seq));
                 CREATE TABLE tokens(
                    token_id TEXT PRIMARY KEY, domain TEXT NOT NULL, type TEXT NOT NULL,
                    produced_by TEXT NOT NULL, thing_ref TEXT, depends_on TEXT NOT NULL,
                    produced_at TEXT NOT NULL, status TEXT NOT NULL,
                    exhausted_by TEXT, exhausted_at TEXT);",
            )
            .await
            .unwrap();
        }
        // 重新打开 → 自愈：DROP 旧表 + 新 schema
        let idx = TursoIndexStore::open(&path).await.unwrap();
        let (k, events) = sample("dom");
        idx.rebuild(&[(
            k.clone(),
            events
                .iter()
                .map(|e| serde_json::from_value(serde_json::to_value(e).unwrap()).unwrap())
                .collect::<Vec<Event>>(),
        )])
        .await
        .unwrap();
        assert_eq!(idx.last_event_seq(&k).await.unwrap(), Some(4));
    }
}
