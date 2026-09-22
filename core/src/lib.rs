use std::fmt;
use std::path::Path;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use refinery::embed_migrations;
use rusqlite::Connection;

use chrono::prelude::*;

use log::{error, info};

pub mod error;
pub mod sftp_connection;

embed_migrations!("migrations");

pub fn configure_sqlite_connection(conn: &Connection) -> Result<(), String> {
    conn.busy_timeout(Duration::from_secs(30))
        .map_err(|e| format!("Error configuring SQLite busy timeout: {e}"))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| format!("Error enabling SQLite foreign keys: {e}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("Error enabling SQLite WAL journal mode: {e}"))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| format!("Error configuring SQLite synchronous mode: {e}"))?;

    Ok(())
}

pub fn open_sqlite_database<P: AsRef<Path>>(path: P) -> Result<Connection, String> {
    let mut conn =
        Connection::open(path).map_err(|e| format!("Error opening Cortex SQLite database: {e}"))?;
    configure_sqlite_connection(&conn)?;
    run_migrations(&mut conn)?;

    Ok(conn)
}

pub fn run_migrations(conn: &mut Connection) -> Result<(), String> {
    migrations::runner()
        .run(conn)
        .map(|_| ())
        .map_err(|e| format!("Error running Cortex migrations: {e}"))
}

/// The set of commands that can be sent over the command queue
#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct SftpDownload {
    pub id: i64,
    pub created: DateTime<Utc>,
    pub size: Option<u64>,
    pub sftp_source: String,
    pub path: String,
    pub remove: bool,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct HttpDownload {
    pub created: DateTime<Utc>,
    pub size: Option<u64>,
    pub url: String,
}

impl fmt::Display for SftpDownload {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.size {
            Some(s) => write!(
                f,
                "SftpDownload({}, {}, {}, {})",
                self.created, s, self.sftp_source, self.path
            ),
            None => write!(
                f,
                "SftpDownload({}, {}, {})",
                self.created, self.sftp_source, self.path
            ),
        }
    }
}

impl fmt::Display for HttpDownload {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.size {
            Some(s) => write!(f, "HttpDownload({}, {}, {})", self.created, s, self.url),
            None => write!(f, "HttpDownload({}, {})", self.created, self.url),
        }
    }
}

/// Wait for a thread to finish, log error or success, ignoring the success
/// value.
pub fn wait_for<T>(join_handle: thread::JoinHandle<T>, thread_name: &str) {
    let join_result = join_handle.join();

    match join_result {
        Ok(_) => {
            info!("{} thread stopped", thread_name);
        }
        Err(e) => {
            error!("{} thread stopped with error: {:?}", thread_name, e);
        }
    }
}

pub type StopCmd = Box<dyn FnOnce() + Send + 'static>;
