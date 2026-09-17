use chrono::prelude::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::base_types::FileInfo;

#[derive(thiserror::Error, Debug)]
pub enum PersistenceError {
    #[error("{message}")]
    Logical { message: String },
}

pub trait Persistence {
    fn delete_sftp_download_file(&self, id: i64) -> Result<(), PersistenceError>;
    fn set_sftp_download_file(&self, id: i64, file_id: i64) -> Result<(), PersistenceError>;
    fn insert_file(
        &self,
        source: &str,
        path: &str,
        modified: &DateTime<Utc>,
        size: i64,
        hash: Option<String>,
    ) -> Result<i64, PersistenceError>;
    fn get_file(&self, source: &str, path: &str) -> Result<Option<FileInfo>, PersistenceError>;
}

#[derive(Clone)]
pub struct SqlitePersistence {
    conn: Arc<Mutex<Connection>>,
}

impl SqlitePersistence {
    pub fn from_arc(conn: Arc<Mutex<Connection>>) -> SqlitePersistence {
        SqlitePersistence { conn }
    }

    pub fn enforce_retention(&self, modifier: &str) -> Result<(), PersistenceError> {
        self.enforce_retention_with_timeout(modifier, Duration::from_secs(30))
    }

    fn enforce_retention_with_timeout(
        &self,
        modifier: &str,
        timeout: Duration,
    ) -> Result<(), PersistenceError> {
        let mut conn = self.conn.lock().map_err(|e| PersistenceError::Logical {
            message: format!("Mutex lock failed: {e}"),
        })?;
        let started = Instant::now();
        conn.progress_handler(1_000, Some(move || started.elapsed() >= timeout))
            .map_err(|e| PersistenceError::Logical {
                message: format!("Could not install retention timeout: {e}"),
            })?;

        let result = (|| {
            let tx = conn.transaction().map_err(|e| PersistenceError::Logical {
                message: format!("Begin transaction failed: {e}"),
            })?;

            let tables = ["dispatched", "sftp_download", "directory_source", "file"];

            for table in &tables {
                let sql = format!("delete from {} where timestamp < datetime('now', ?)", table);
                tx.execute(&sql, params![modifier])
                    .map_err(|e| PersistenceError::Logical {
                        message: format!("Error deleting from {}: {}", table, e),
                    })?;
            }

            tx.commit().map_err(|e| PersistenceError::Logical {
                message: format!("Commit failed: {e}"),
            })
        })();

        conn.progress_handler(0, None::<fn() -> bool>)
            .map_err(|e| PersistenceError::Logical {
                message: format!("Could not clear retention timeout: {e}"),
            })?;

        result
    }
}

impl Persistence for SqlitePersistence {
    fn set_sftp_download_file(&self, id: i64, file_id: i64) -> Result<(), PersistenceError> {
        let conn = self.conn.lock().map_err(|e| PersistenceError::Logical {
            message: format!("Mutex lock failed: {e}"),
        })?;
        conn.execute(
            "update sftp_download set file_id = ?2 where id = ?1",
            params![id, file_id],
        )
        .map(|_| ())
        .map_err(|e| PersistenceError::Logical {
            message: format!("Error updating sftp_download: {e}"),
        })
    }

    fn delete_sftp_download_file(&self, id: i64) -> Result<(), PersistenceError> {
        let conn = self.conn.lock().map_err(|e| PersistenceError::Logical {
            message: format!("Mutex lock failed: {e}"),
        })?;
        conn.execute("delete from sftp_download where id = ?1", params![id])
            .map(|_| ())
            .map_err(|e| PersistenceError::Logical {
                message: format!("Error deleting sftp_download: {e}"),
            })
    }

    fn insert_file(
        &self,
        source: &str,
        path: &str,
        modified: &DateTime<Utc>,
        size: i64,
        hash: Option<String>,
    ) -> Result<i64, PersistenceError> {
        let conn = self.conn.lock().map_err(|e| PersistenceError::Logical {
            message: format!("Mutex lock failed: {e}"),
        })?;
        let modified_str = modified.to_rfc3339();
        let mut stmt = conn
            .prepare(
                "insert into file (source, path, modified, size, hash)
                 values (?1, ?2, ?3, ?4, ?5)
                 on conflict(source, path) do update set
                   modified=excluded.modified, size=excluded.size, hash=excluded.hash
                 returning id",
            )
            .map_err(|e| PersistenceError::Logical {
                message: format!("Prepare insert file failed: {e}"),
            })?;

        let id: i64 = stmt
            .query_row(params![source, path, modified_str, size, hash], |row| {
                row.get(0)
            })
            .map_err(|e| PersistenceError::Logical {
                message: format!("Insert file failed: {e}"),
            })?;

        Ok(id)
    }

    fn get_file(&self, source: &str, path: &str) -> Result<Option<FileInfo>, PersistenceError> {
        let conn = self.conn.lock().map_err(|e| PersistenceError::Logical {
            message: format!("Mutex lock failed: {e}"),
        })?;
        let mut stmt = conn
            .prepare("select modified, size, hash from file where source = ?1 and path = ?2")
            .map_err(|e| PersistenceError::Logical {
                message: format!("Prepare select file failed: {e}"),
            })?;

        let row = stmt
            .query_row(params![source, path], |row| {
                let modified_str: String = row.get(0)?;
                let modified = modified_str.parse::<DateTime<Utc>>().map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                let size: i64 = row.get(1)?;
                let hash: Option<String> = row.get(2)?;
                Ok(FileInfo {
                    modified,
                    size,
                    hash,
                })
            })
            .optional()
            .map_err(|e| PersistenceError::Logical {
                message: format!("Select file failed: {e}"),
            })?;

        Ok(row)
    }
}

#[derive(Clone)]
pub struct SqliteAsyncPersistence {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteAsyncPersistence {
    pub fn new(conn: Arc<Mutex<Connection>>) -> SqliteAsyncPersistence {
        SqliteAsyncPersistence { conn }
    }

    pub async fn insert_dispatched(
        &self,
        dest: &str,
        file_id: i64,
    ) -> Result<(), PersistenceError> {
        let conn = self.conn.clone();
        let dest = dest.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| PersistenceError::Logical {
                message: format!("Mutex lock failed: {e}"),
            })?;
            conn.execute(
                "insert into dispatched (file_id, target, timestamp) values (?1, ?2, datetime('now'))",
                params![file_id, dest],
            )
            .map(|_| ())
            .map_err(|e| PersistenceError::Logical {
                message: format!("Error inserting dispatched: {e}"),
            })
        })
        .await
        .map_err(|e| PersistenceError::Logical {
            message: format!("Join error inserting dispatched: {e}"),
        })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_add_file_foreign_key_indexes() {
        let mut conn = Connection::open_in_memory().unwrap();
        cortex_core::run_migrations(&mut conn).unwrap();

        for index in [
            "dispatched_file_id_idx",
            "directory_source_file_id_idx",
            "sftp_download_file_id_idx",
        ] {
            let exists: bool = conn
                .query_row(
                    "select exists(select 1 from sqlite_master where type = 'index' and name = ?)",
                    [index],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "migration did not create {index}");
        }
    }

    #[test]
    fn timed_out_retention_rolls_back_and_releases_connection() {
        let mut conn = Connection::open_in_memory().unwrap();
        cortex_core::run_migrations(&mut conn).unwrap();
        conn.execute(
            "insert into file (timestamp, source, path, modified, size) values (datetime('now', '-2 days'), 'source', 'path', datetime('now'), 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "with recursive rows(value) as (select 1 union all select value + 1 from rows where value < 10000) insert into dispatched (file_id, target, timestamp) select 1, 'target', datetime('now', '-2 days') from rows",
            [],
        )
        .unwrap();

        let persistence = SqlitePersistence::from_arc(Arc::new(Mutex::new(conn)));
        assert!(
            persistence
                .enforce_retention_with_timeout("-1 days", Duration::ZERO)
                .is_err()
        );

        {
            let conn = persistence.conn.lock().unwrap();
            let file_count: i64 = conn
                .query_row("select count(*) from file", [], |row| row.get(0))
                .unwrap();
            let dispatched_count: i64 = conn
                .query_row("select count(*) from dispatched", [], |row| row.get(0))
                .unwrap();
            assert_eq!(file_count, 1);
            assert_eq!(dispatched_count, 10000);
        }

        let file_id = persistence
            .insert_file("source", "new-path", &Utc::now(), 1, None)
            .unwrap();
        assert!(file_id > 0);
        assert!(
            persistence
                .get_file("source", "new-path")
                .unwrap()
                .is_some()
        );

        persistence
            .enforce_retention_with_timeout("-1 days", Duration::from_secs(1))
            .unwrap();
        assert!(persistence.get_file("source", "path").unwrap().is_none());
    }
}
