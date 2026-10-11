//! The supervisor is the sole owner/writer of this connection. Ledger writes
//! precede Running; records are removed only after verified session emptiness.
use devdesk_term::kill::{OwnedSession, ProcessIdentity};
use rusqlite::{Connection, OptionalExtension, params};
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
        if version > 4 {
            return Err("state.db was created by a newer DevDesk; refusing to change it".into());
        }
        if version == 0 {
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS session_ledger (sid INTEGER PRIMARY KEY, leader_started_at INTEGER NOT NULL, launch_id TEXT NOT NULL, owner TEXT NOT NULL); PRAGMA user_version=1; COMMIT;").map_err(|e|e.to_string())?;
        }
        if version < 2 {
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE agents (id TEXT PRIMARY KEY, record TEXT NOT NULL); CREATE TABLE ui_state (id INTEGER PRIMARY KEY CHECK(id=1), layout TEXT NOT NULL); PRAGMA user_version=2; COMMIT;").map_err(|e|e.to_string())?;
        }
        if version < 3 {
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE prompts (id TEXT PRIMARY KEY, title TEXT NOT NULL, body TEXT NOT NULL, revision INTEGER NOT NULL, updated_at INTEGER NOT NULL); CREATE INDEX prompts_recent ON prompts(updated_at DESC, id); PRAGMA user_version=3; COMMIT;").map_err(|e|e.to_string())?;
        }
        if version < 4 {
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE meetings (id TEXT PRIMARY KEY, started_at INTEGER NOT NULL, record TEXT NOT NULL); CREATE INDEX meetings_recent ON meetings(started_at DESC, id); PRAGMA user_version=4; COMMIT;").map_err(|e|e.to_string())?;
        }
        Ok(Self { connection })
    }
    pub fn save_meeting(&self, meeting: &mut crate::meetings::Meeting) -> Result<(), String> {
        meeting.revision = self
            .meeting(&meeting.id)?
            .map(|previous| previous.revision.saturating_add(1))
            .unwrap_or(1);
        let record = serde_json::to_string(meeting).map_err(|e| e.to_string())?;
        self.connection.execute("INSERT INTO meetings(id,started_at,record) VALUES (?1,?2,?3) ON CONFLICT(id) DO UPDATE SET record=excluded.record", params![meeting.id, meeting.started_at, record]).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn meetings(&self) -> Result<Vec<crate::meetings::Meeting>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT record FROM meetings ORDER BY started_at DESC,id")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.map(|row| {
            row.map_err(|e| e.to_string())
                .and_then(|record| serde_json::from_str(&record).map_err(|e| e.to_string()))
        })
        .collect()
    }
    pub fn meeting(&self, id: &str) -> Result<Option<crate::meetings::Meeting>, String> {
        let record: Option<String> = self
            .connection
            .query_row("SELECT record FROM meetings WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        record
            .map(|value| serde_json::from_str(&value).map_err(|e| e.to_string()))
            .transpose()
    }
    pub fn delete_meeting(&self, id: &str) -> Result<(), String> {
        self.connection
            .execute("DELETE FROM meetings WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn list_prompts(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<crate::prompts::Summary>, String> {
        let limit = limit.clamp(1, 100);
        let offset = offset.min(100_000);
        let query = query.trim();
        let mut statement = self.connection.prepare("SELECT id,title,revision,updated_at FROM prompts WHERE (?1='' OR instr(lower(title),lower(?1))>0 OR instr(lower(body),lower(?1))>0) ORDER BY updated_at DESC,id LIMIT ?2 OFFSET ?3").map_err(|e|e.to_string())?;
        let rows = statement
            .query_map(params![query, limit as i64, offset as i64], |row| {
                Ok(crate::prompts::Summary {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    revision: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.map(|row| row.map_err(|e| e.to_string())).collect()
    }
    pub fn read_prompt(&self, id: &str) -> Result<Option<crate::prompts::Prompt>, String> {
        self.connection
            .query_row(
                "SELECT id,title,body,revision,updated_at FROM prompts WHERE id=?1",
                [id],
                |row| {
                    Ok(crate::prompts::Prompt {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        body: row.get(2)?,
                        revision: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())
    }
    pub fn write_prompt(
        &self,
        write: &crate::prompts::Write,
    ) -> Result<crate::prompts::Prompt, String> {
        write.validate()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs() as i64;
        let id = if let Some(id) = &write.id {
            let affected = self.connection.execute("UPDATE prompts SET title=?1,body=?2,revision=revision+1,updated_at=?3 WHERE id=?4 AND revision=?5", params![write.title.trim(),write.body,now,id,write.expected_revision]).map_err(|e|e.to_string())?;
            if affected == 0 {
                return Err("Prompt changed or was deleted; reload before saving".into());
            }
            id.clone()
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            self.connection
                .execute(
                    "INSERT INTO prompts(id,title,body,revision,updated_at) VALUES (?1,?2,?3,1,?4)",
                    params![id, write.title.trim(), write.body, now],
                )
                .map_err(|e| e.to_string())?;
            id
        };
        self.read_prompt(&id)?
            .ok_or("Saved prompt could not be read".into())
    }
    pub fn delete_prompt(&self, id: &str, revision: i64) -> Result<(), String> {
        crate::prompts::validate_delete(id, revision)?;
        let affected = self
            .connection
            .execute(
                "DELETE FROM prompts WHERE id=?1 AND revision=?2",
                params![id, revision],
            )
            .map_err(|e| e.to_string())?;
        if affected == 0 {
            return Err("Prompt changed or was deleted; reload before deleting".into());
        }
        Ok(())
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
    /// Read on the actor's connection so a failed journal insert can be retried
    /// without rewriting already persisted identities on each active scan.
    pub fn contains(&self, session: OwnedSession) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM session_ledger WHERE sid=?1 AND leader_started_at=?2)",
                params![
                    session.leader.pid,
                    i64::try_from(session.leader.started_us).map_err(|e| e.to_string())?
                ],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
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
    fn prompts_round_trip_and_reject_stale_updates() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let created = store
            .write_prompt(&crate::prompts::Write {
                id: None,
                title: " Minimal product design ".into(),
                body: "Use generous spacing.".into(),
                expected_revision: None,
            })
            .unwrap();
        assert_eq!(created.title, "Minimal product design");
        assert_eq!(created.revision, 1);
        assert_eq!(store.list_prompts("product", 10, 0).unwrap().len(), 1);
        assert_eq!(store.list_prompts(" SPACING ", 10, 0).unwrap().len(), 1);
        assert!(store.list_prompts("missing", 10, 0).unwrap().is_empty());
        let edit = crate::prompts::Write {
            id: Some(created.id.clone()),
            title: created.title.clone(),
            body: "Use a restrained palette.".into(),
            expected_revision: Some(1),
        };
        let updated = store.write_prompt(&edit).unwrap();
        assert_eq!(updated.revision, 2);
        assert!(store.write_prompt(&edit).is_err());
        assert!(store.delete_prompt(&updated.id, 1).is_err());
        assert_eq!(
            store.read_prompt(&updated.id).unwrap().unwrap().body,
            updated.body
        );
        store.delete_prompt(&updated.id, 2).unwrap();
        assert!(store.read_prompt(&updated.id).unwrap().is_none());
    }

    #[test]
    fn migrates_and_drops_stale_birth_without_offering_unrelated_process() {
        let path = std::env::temp_dir().join(format!("devdesk-ledger-{}.db", std::process::id()));
        let store = Store::open(&path).unwrap();
        let version: u32 = store
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 4);
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
            .pragma_update(None, "user_version", 5)
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
    fn meetings_survive_reopen_and_can_be_deleted() {
        let path =
            std::env::temp_dir().join(format!("devdesk-meetings-{}.db", uuid::Uuid::new_v4()));
        let mut meeting = crate::meetings::Meeting::new(path.with_extension("audio"));
        {
            let store = Store::open(&path).unwrap();
            store.save_meeting(&mut meeting).unwrap();
        }
        {
            let store = Store::open(&path).unwrap();
            let saved = store.meeting(&meeting.id).unwrap().unwrap();
            assert_eq!(saved.audio_path, meeting.audio_path);
            assert_eq!(saved.microphone_path, meeting.microphone_path);
            assert_eq!(saved.status, crate::meetings::Status::Starting);
            assert_eq!(saved.revision, 1);
            let mut revised = saved.clone();
            revised.notes = "## Summary\n\nEdited.".into();
            store.save_meeting(&mut revised).unwrap();
            assert_eq!(revised.revision, 2);
            assert_eq!(store.meeting(&meeting.id).unwrap().unwrap().revision, 2);
            assert_eq!(store.meetings().unwrap().len(), 1);
            store.delete_meeting(&meeting.id).unwrap();
            assert!(store.meeting(&meeting.id).unwrap().is_none());
        }
        std::fs::remove_file(&path).unwrap();
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

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
            launch_args: vec![],
            provider: crate::agent::Provider::Claude,
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
