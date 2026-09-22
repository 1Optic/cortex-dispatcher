use chrono::prelude::*;
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::base_types::FileInfo;

const SQLITE_BUSY_RETRIES: usize = 5;
#[cfg(not(test))]
const SQLITE_BUSY_RETRY_DELAY: Duration = Duration::from_millis(100);
#[cfg(test)]
const SQLITE_BUSY_RETRY_DELAY: Duration = Duration::ZERO;
const RETENTION_DELETE_BATCH_SIZE: i64 = 1_000;

#[derive(thiserror::Error, Debug)]
pub enum PersistenceError {
    #[error("{message}")]
    Logical { message: String },
}

fn is_transient_sqlite_lock(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(err, _)
            if matches!(err.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

fn retry_sqlite_write<T, F>(mut operation: F) -> Result<T, rusqlite::Error>
where
    F: FnMut() -> Result<T, rusqlite::Error>,
{
    for attempt in 0..=SQLITE_BUSY_RETRIES {
        match operation() {
            Ok(value) => return Ok(value),
            Err(e) if is_transient_sqlite_lock(&e) && attempt < SQLITE_BUSY_RETRIES => {
                thread::sleep(SQLITE_BUSY_RETRY_DELAY);
            }
            Err(e) => return Err(e),
        }
    }

    unreachable!("retry loop always returns before exhausting attempts")
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
            let tables = ["dispatched", "sftp_download", "directory_source", "file"];

            for table in &tables {
                loop {
                    if started.elapsed() >= timeout {
                        return Err(PersistenceError::Logical {
                            message: "Retention enforcement timed out".to_string(),
                        });
                    }

                    let sql = format!(
                        "delete from {} where rowid in (select rowid from {} where timestamp < datetime('now', ?) limit ?)",
                        table, table
                    );
                    let deleted = retry_sqlite_write(|| {
                        let tx = conn.transaction()?;
                        let deleted =
                            tx.execute(&sql, params![modifier, RETENTION_DELETE_BATCH_SIZE])?;
                        tx.commit()?;
                        Ok(deleted)
                    })
                    .map_err(|e| PersistenceError::Logical {
                        message: format!("Error deleting from {}: {}", table, e),
                    })?;

                    if deleted < RETENTION_DELETE_BATCH_SIZE as usize {
                        break;
                    }
                }
            }

            Ok(())
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
        retry_sqlite_write(|| {
            conn.execute(
                "update sftp_download set file_id = ?2 where id = ?1",
                params![id, file_id],
            )
        })
        .map(|_| ())
        .map_err(|e| PersistenceError::Logical {
            message: format!("Error updating sftp_download: {e}"),
        })
    }

    fn delete_sftp_download_file(&self, id: i64) -> Result<(), PersistenceError> {
        let conn = self.conn.lock().map_err(|e| PersistenceError::Logical {
            message: format!("Mutex lock failed: {e}"),
        })?;
        retry_sqlite_write(|| conn.execute("delete from sftp_download where id = ?1", params![id]))
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

        let id: i64 = retry_sqlite_write(|| {
            stmt.query_row(params![source, path, modified_str, size, hash], |row| {
                row.get(0)
            })
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
            retry_sqlite_write(|| {
                conn.execute(
                    "insert into dispatched (file_id, target, timestamp) values (?1, ?2, datetime('now'))",
                    params![file_id, dest],
                )
            })
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
    use rusqlite::ffi;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sqlite_failure(code: ErrorCode) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(
            ffi::Error {
                code,
                extended_code: 0,
            },
            None,
        )
    }

    fn temp_db_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "cortex-dispatcher-{name}-{}-{nanos}.db",
            std::process::id()
        ))
    }

    fn remove_sqlite_files(path: &PathBuf) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(path.with_extension("db-wal"));
        let _ = fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn open_sqlite_database_configures_pragmas_and_runs_migrations() {
        let path = temp_db_path("connection");
        let conn = cortex_core::open_sqlite_database(&path).unwrap();

        let busy_timeout: i64 = conn
            .query_row("pragma busy_timeout", [], |row| row.get(0))
            .unwrap();
        let foreign_keys: i64 = conn
            .query_row("pragma foreign_keys", [], |row| row.get(0))
            .unwrap();
        let journal_mode: String = conn
            .query_row("pragma journal_mode", [], |row| row.get(0))
            .unwrap();
        let synchronous: i64 = conn
            .query_row("pragma synchronous", [], |row| row.get(0))
            .unwrap();
        let file_table_exists: bool = conn
            .query_row(
                "select exists(select 1 from sqlite_master where type = 'table' and name = 'file')",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(busy_timeout, 30_000);
        assert_eq!(foreign_keys, 1);
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        assert_eq!(synchronous, 1);
        assert!(file_table_exists);

        drop(conn);
        remove_sqlite_files(&path);
    }

    #[test]
    fn retry_sqlite_write_retries_database_busy_until_success() {
        let mut attempts = 0;

        let result = retry_sqlite_write(|| {
            attempts += 1;
            if attempts < 3 {
                Err(sqlite_failure(ErrorCode::DatabaseBusy))
            } else {
                Ok("ok")
            }
        })
        .unwrap();

        assert_eq!(result, "ok");
        assert_eq!(attempts, 3);
    }

    #[test]
    fn retry_sqlite_write_retries_database_locked_until_success() {
        let mut attempts = 0;

        let result = retry_sqlite_write(|| {
            attempts += 1;
            if attempts < 3 {
                Err(sqlite_failure(ErrorCode::DatabaseLocked))
            } else {
                Ok("ok")
            }
        })
        .unwrap();

        assert_eq!(result, "ok");
        assert_eq!(attempts, 3);
    }

    #[test]
    fn retry_sqlite_write_stops_after_busy_retry_limit() {
        let mut attempts = 0;

        let error = retry_sqlite_write(|| -> Result<(), rusqlite::Error> {
            attempts += 1;
            Err(sqlite_failure(ErrorCode::DatabaseBusy))
        })
        .unwrap_err();

        assert!(is_transient_sqlite_lock(&error));
        assert_eq!(attempts, SQLITE_BUSY_RETRIES + 1);
    }

    #[test]
    fn retry_sqlite_write_does_not_retry_non_lock_errors() {
        let mut attempts = 0;

        let error = retry_sqlite_write(|| -> Result<(), rusqlite::Error> {
            attempts += 1;
            Err(sqlite_failure(ErrorCode::ConstraintViolation))
        })
        .unwrap_err();

        assert!(!is_transient_sqlite_lock(&error));
        assert_eq!(attempts, 1);
    }

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
