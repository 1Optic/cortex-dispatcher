use chrono::prelude::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::{Arc, Mutex};

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
}

impl Persistence for SqlitePersistence {
    fn set_sftp_download_file(&self, id: i64, file_id: i64) -> Result<(), PersistenceError> {
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
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
            let conn = conn.lock().unwrap();
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
