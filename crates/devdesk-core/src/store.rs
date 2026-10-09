//! The supervisor is the sole owner/writer of this connection. Ledger writes
//! precede Running; records are removed only after verified session emptiness.
use devdesk_term::kill::{OwnedSession, ProcessIdentity};
use rusqlite::{Connection, params};
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Orphan {
    pub session: OwnedSession,
    pub owner: String,
    pub members: usize,
}
pub struct Store {
    connection: Connection,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        let connection = Connection::open(path).map_err(|e| e.to_string())?;
        connection
            .busy_timeout(Duration::from_millis(250))
            .map_err(|e| e.to_string())?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| e.to_string())?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(|e| e.to_string())?;
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|e| e.to_string())?;
        if version > 2 {
            return Err("state.db was created by a newer DevDesk; refusing to change it".into());
        }
        if version == 0 {
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS session_ledger (sid INTEGER PRIMARY KEY, leader_started_at INTEGER NOT NULL, launch_id TEXT NOT NULL, owner TEXT NOT NULL); PRAGMA user_version=1; COMMIT;").map_err(|e|e.to_string())?;
        }
        if version < 2 {
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE agents (id TEXT PRIMARY KEY, record TEXT NOT NULL); CREATE TABLE ui_state (id INTEGER PRIMARY KEY CHECK(id=1), layout TEXT NOT NULL); PRAGMA user_version=2; COMMIT;").map_err(|e|e.to_string())?;
        }
        Ok(Self { connection })
    }
    pub fn save_agent(&self, record: &crate::agent::Record) -> Result<(), String> {
        let json = serde_json::to_string(record).map_err(|e| e.to_string())?;
        self.connection.execute("INSERT INTO agents(id,record) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET record=excluded.record",params![record.id,json]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn delete_agent(&self, id: &str) -> Result<(), String> {
        self.connection
            .execute(
                "DELETE FROM agents WHERE id=?1 OR json_extract(record, '$.parent')=?1",
                [id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn delete_project_agents(&self, project: &str) -> Result<(), String> {
        let records = self.agents()?;
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        for record in records
            .into_iter()
            .filter(|record| record.project == project)
        {
            transaction
                .execute("DELETE FROM agents WHERE id=?1", [&record.id])
                .map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())
    }
    pub fn agents(&self) -> Result<Vec<crate::agent::Record>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT record FROM agents ORDER BY rowid")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|row| {
            serde_json::from_str(&row.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
        })
        .collect()
    }
    pub fn save_layout(&self, layout: &crate::layout::Layout) -> Result<(), String> {
        let json = serde_json::to_string(layout).map_err(|e| e.to_string())?;
        self.connection.execute("INSERT INTO ui_state VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET layout=excluded.layout",params![json]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn layout(&self) -> Result<Option<crate::layout::Layout>, String> {
        use rusqlite::OptionalExtension;
        let json: Option<String> = self
            .connection
            .query_row("SELECT layout FROM ui_state WHERE id=1", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        json.map(|json| serde_json::from_str(&json).map_err(|e| e.to_string()))
            .transpose()
    }
    pub fn insert(&self, session: OwnedSession, launch: u64, owner: &str) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT OR REPLACE INTO session_ledger VALUES (?1,?2,?3,?4)",
                params![
                    session.leader.pid,
                    i64::try_from(session.leader.started_us).map_err(|e| e.to_string())?,
                    launch.to_string(),
                    owner
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn remove(&self, session: OwnedSession) -> Result<(), String> {
        self.connection
            .execute(
                "DELETE FROM session_ledger WHERE sid=?1 AND leader_started_at=?2",
                params![
                    session.leader.pid,
                    i64::try_from(session.leader.started_us).map_err(|e| e.to_string())?
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn survivors(&self) -> Result<Vec<Orphan>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT sid,leader_started_at,owner FROM session_ledger ORDER BY sid")
            .map_err(|e| e.to_string())?;
        let records = statement
            .query_map([], |row| {
                Ok((
                    OwnedSession {
                        leader: ProcessIdentity {
                            pid: row.get(0)?,
                            started_us: u64::try_from(row.get::<_, i64>(1)?).map_err(|error| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    1,
                                    rusqlite::types::Type::Integer,
                                    Box::new(error),
                                )
                            })?,
                        },
                    },
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut survivors = vec![];
        for record in records {
            let (session, owner) = record.map_err(|e| e.to_string())?;
            let members = session
                .members()
                .map_err(|e| format!("Cannot verify prior session: {e}"))?;
            if members.is_empty() {
                self.remove(session)?;
            } else {
                survivors.push(Orphan {
                    session,
                    owner,
                    members: members.len(),
                });
            }
        }
        Ok(survivors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_and_drops_stale_birth_without_offering_unrelated_process() {
        let path = std::env::temp_dir().join(format!("devdesk-ledger-{}.db", std::process::id()));
        let store = Store::open(&path).unwrap();
        let version: u32 = store
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);
        let current = ProcessIdentity::read(std::process::id() as i32).unwrap();
        // A stale record points at this live test process, but carries a
        // different birth time. Recovery must never offer it as an orphan.
        let stale = OwnedSession {
            leader: ProcessIdentity {
                started_us: current.started_us + 1,
                ..current
            },
        };
        store.insert(stale, 1, "stale").unwrap();
        assert!(store.survivors().unwrap().is_empty());
        assert_eq!(ProcessIdentity::read(current.pid).unwrap(), current);
        let count: u32 = store
            .connection
            .query_row("SELECT COUNT(*) FROM session_ledger", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
        store
            .connection
            .pragma_update(None, "user_version", 3)
            .unwrap();
        drop(store);
        assert!(Store::open(&path).err().unwrap().contains("newer DevDesk"));
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn migrates_v1_preserves_ledger_and_roundtrips_role_cwd_and_layout() {
        let path =
            std::env::temp_dir().join(format!("devdesk-m4-migration-{}.db", std::process::id()));
        let old = Connection::open(&path).unwrap();
        old.execute_batch("CREATE TABLE session_ledger(sid INTEGER PRIMARY KEY,leader_started_at INTEGER NOT NULL,launch_id TEXT NOT NULL,owner TEXT NOT NULL); INSERT INTO session_ledger VALUES(123,456,'1','old'); PRAGMA user_version=1;").unwrap();
        drop(old);
        let store = Store::open(&path).unwrap();
        let owner: String = store
            .connection
            .query_row("SELECT owner FROM session_ledger", [], |row| row.get(0))
            .unwrap();
        assert_eq!(owner, "old");
        let record = crate::agent::Record {
            id: uuid::Uuid::new_v4().to_string(),
            project: "p".into(),
            name: "Agent".into(),
            cwd: PathBuf::from("/tmp/record-cwd"),
            role: crate::agent::Role::Side,
            parent: None,
            snapshot_at: None,
            session_id: Some(uuid::Uuid::new_v4().to_string()),
        };
        store.save_agent(&record).unwrap();
        let layout = crate::layout::Layout {
            active_project: Some("p".into()),
            tabs: vec![crate::layout::Tab {
                key: "agent".into(),
                project: "p".into(),
                name: "Agent".into(),
                kind: crate::layout::Kind::Agent(record.id.clone()),
                cwd: None,
                open: true,
            }],
            ..Default::default()
        };
        store.save_layout(&layout).unwrap();
        drop(store);
        let reopened = Store::open(&path).unwrap();
        let saved = reopened.agents().unwrap().remove(0);
        assert_eq!(saved.cwd, record.cwd);
        assert_eq!(saved.role, crate::agent::Role::Side);
        assert_eq!(saved.session_id, record.session_id);
        assert_eq!(reopened.layout().unwrap().unwrap(), layout);
        drop(reopened);
        std::fs::remove_file(path).unwrap();
    }
    use std::path::PathBuf;
}
