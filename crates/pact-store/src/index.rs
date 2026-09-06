//! 派生索引（D13 隔离层）：`IndexStore` trait + turso 默认实现。
//!
//! 索引永远可丢弃重建——真相源是 JSONL 事件日志（设计 §7.4）。
//! schema 相对设计稿的修正：events 主键为 (domain, seq)——
//! seq 是域内单调，跨域会碰撞（记录于 docs/design-change.md）。

use crate::error::StoreError;
use crate::event::{apply_event, Event};
use async_trait::async_trait;
use pact_core::{CounterState, ParticipantName, ThingRef, Timestamp, Token};
use std::path::Path;
use std::sync::Arc;
use turso::{Builder, Connection, Value};

/// token 当前状态（tokens 表的 status 派生列）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenStatus {
    Counter,
    Archived { exhausted_by: ParticipantName, exhausted_at: Timestamp },
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
    async fn upsert_event(&self, domain: &str, ev: &Event) -> Result<(), StoreError>;
    /// 写入/更新 token 行（status = counter | archived）。
    async fn upsert_token(
        &self,
        domain: &str,
        t: &Token,
        status: &TokenStatus,
    ) -> Result<(), StoreError>;
    /// 域内最大事件 seq（None = 该域无事件）。启动一致性校验用。
    async fn last_event_seq(&self, domain: &str) -> Result<Option<u64>, StoreError>;
    /// 从全部域日志全量重建（清空后重放）。非事务性——索引可随时重建，无需原子。
    async fn rebuild(&self, entries: &[(String, Vec<Event>)]) -> Result<(), StoreError>;
}

fn to_index_err(e: turso::Error) -> StoreError {
    StoreError::Index(e.to_string())
}

/// turso（原 Limbo）实现。BETA 风险由"可随时重建"对冲（D13）。
pub struct TursoIndexStore {
    conn: Arc<Connection>,
}

#[async_trait]
impl IndexStore for TursoIndexStore {
    async fn open(path: &Path) -> Result<Self, StoreError> {
        let db = Builder::new_local(path.to_string_lossy().as_ref())
            .build()
            .await
            .map_err(to_index_err)?;
        let conn = db.connect().map_err(to_index_err)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events(
                domain TEXT NOT NULL,
                seq INTEGER NOT NULL,
                kind TEXT NOT NULL,
                ts TEXT NOT NULL,
                payload TEXT NOT NULL,
                PRIMARY KEY(domain, seq));
             CREATE TABLE IF NOT EXISTS tokens(
                token_id TEXT PRIMARY KEY,
                domain TEXT NOT NULL,
                type TEXT NOT NULL,
                produced_by TEXT NOT NULL,
                thing_ref TEXT,
                depends_on TEXT NOT NULL,
                produced_at TEXT NOT NULL,
                status TEXT NOT NULL,
                exhausted_by TEXT,
                exhausted_at TEXT);
             CREATE INDEX IF NOT EXISTS idx_tokens_domain_type ON tokens(domain, type, status);",
        )
        .await
        .map_err(to_index_err)?;
        Ok(Self { conn: Arc::new(conn) })
    }

    async fn upsert_event(&self, domain: &str, ev: &Event) -> Result<(), StoreError> {
        let payload = serde_json::to_string(ev)?;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO events(domain, seq, kind, ts, payload)
                 VALUES(?1, ?2, ?3, ?4, ?5)",
                [
                    Value::Text(domain.to_owned()),
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
        domain: &str,
        t: &Token,
        status: &TokenStatus,
    ) -> Result<(), StoreError> {
        let depends_on = serde_json::to_string(&t.depends_on)?;
        let thing_ref = t.thing_ref.0.clone();
        let (exhausted_by, exhausted_at) = match status {
            TokenStatus::Counter => (None, None),
            TokenStatus::Archived { exhausted_by, exhausted_at } => {
                (Some(exhausted_by.0.clone()), Some(exhausted_at.clone()))
            }
        };
        self.conn
            .execute(
                "INSERT OR REPLACE INTO tokens(
                    token_id, domain, type, produced_by, thing_ref, depends_on,
                    produced_at, status, exhausted_by, exhausted_at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                [
                    Value::Text(t.id.0.clone()),
                    Value::Text(domain.to_owned()),
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

    async fn last_event_seq(&self, domain: &str) -> Result<Option<u64>, StoreError> {
        let mut rows = self
            .conn
            .query(
                "SELECT MAX(seq) FROM events WHERE domain = ?1",
                [Value::Text(domain.to_owned())],
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

    async fn rebuild(&self, entries: &[(String, Vec<Event>)]) -> Result<(), StoreError> {
        // 清空后重放写入。非事务：索引是纯派生物，中断只会留下部分索引，
        // 下次启动的一致性校验会再次触发重建（自愈）。
        self.conn.execute("DELETE FROM events", ()).await.map_err(to_index_err)?;
        self.conn.execute("DELETE FROM tokens", ()).await.map_err(to_index_err)?;

        for (domain, events) in entries {
            // 模拟重放得到 token 终态（counter | archived + 消费者/时间）
            let mut state = CounterState::new();
            for ev in events {
                apply_event(&mut state, ev)?;
                match ev {
                    Event::Seeded { token, .. } | Event::Produced { token, .. } => {
                        self.upsert_token(domain, token, &TokenStatus::Counter).await?;
                    }
                    Event::Consumed { token_id, by, ts, .. } => {
                        // apply_event 已保证目标在 Counter 上并移入 Archive；终态从 Archive 取
                        if let Some(entry) =
                            state.archive.iter().rev().find(|e| e.token.id == *token_id)
                        {
                            self.upsert_token(
                                domain,
                                &entry.token,
                                &TokenStatus::Archived {
                                    exhausted_by: by.clone(),
                                    exhausted_at: ts.clone(),
                                },
                            )
                            .await?;
                        }
                    }
                    _ => {}
                }
                self.upsert_event(domain, ev).await?;
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

    fn sample(domain: &str) -> (String, Vec<Event>) {
        (
            domain.to_owned(),
            vec![
                Event::DomainLoaded {
                    seq: 1,
                    ts: "T0".into(),
                    name: domain.into(),
                    declaration: "…".into(),
                },
                Event::Seeded { seq: 2, ts: "T1".into(), token: token(&format!("{domain}-t1"), "req") },
                Event::Produced {
                    seq: 3,
                    ts: "T2".into(),
                    token: token(&format!("{domain}-t2"), "design"),
                },
                Event::Consumed {
                    seq: 4,
                    ts: "T3".into(),
                    token_id: TokenId(format!("{domain}-t1")),
                    by: ParticipantName("arch".into()),
                },
            ],
        )
    }

    #[tokio::test]
    async fn upsert_and_last_event_seq() {
        let dir = tempfile::tempdir().unwrap();
        let idx = TursoIndexStore::open(&dir.path().join("index.tursodb")).await.unwrap();
        let (_, events) = sample("dom");
        for ev in &events {
            idx.upsert_event("dom", ev).await.unwrap();
        }
        assert_eq!(idx.last_event_seq("dom").await.unwrap(), Some(4));
        assert_eq!(idx.last_event_seq("other").await.unwrap(), None);
    }

    #[tokio::test]
    async fn rebuild_projects_token_status() {
        let dir = tempfile::tempdir().unwrap();
        let idx = TursoIndexStore::open(&dir.path().join("index.tursodb")).await.unwrap();
        let entries = vec![sample("dom"), sample("dom2")];
        idx.rebuild(&entries).await.unwrap();
        assert_eq!(idx.last_event_seq("dom").await.unwrap(), Some(4));
        assert_eq!(idx.last_event_seq("dom2").await.unwrap(), Some(4));

        // tokens 投影：dom 的 t1 归档、t2 在 Counter
        let mut rows = idx
            .conn
            .query(
                "SELECT status, exhausted_by FROM tokens WHERE domain = 'dom' ORDER BY token_id",
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
}
