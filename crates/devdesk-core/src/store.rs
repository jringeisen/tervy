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
        if version > 1 {
            return Err("state.db was created by a newer DevDesk; refusing to change it".into());
        }
        if version == 0 {
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS session_ledger (sid INTEGER PRIMARY KEY, leader_started_at INTEGER NOT NULL, launch_id TEXT NOT NULL, owner TEXT NOT NULL); PRAGMA user_version=1; COMMIT;").map_err(|e|e.to_string())?;
        }
        Ok(Self { connection })
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
        assert_eq!(version, 1);
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
            .pragma_update(None, "user_version", 2)
            .unwrap();
        drop(store);
        assert!(Store::open(&path).err().unwrap().contains("newer DevDesk"));
        std::fs::remove_file(path).unwrap();
    }
}
