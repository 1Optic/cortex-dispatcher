use std::convert::TryFrom;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{thread, time};

use crossbeam_channel::{SendTimeoutError, Sender};
use log::{debug, error, info};

use retry::{delay::Fixed, retry, OperationResult};

use chrono::prelude::*;

use anyhow::{anyhow, Result};

use cortex_core::error::DispatcherError;
use cortex_core::sftp_connection::SftpConfig;
use cortex_core::SftpDownload;

use crate::metrics;
use crate::settings::SftpSource;
use rusqlite::{params, Connection};
use std::sync::Mutex;

/// Starts a new thread with an SFTP scanner for the specified source.
///
/// For encountered files to be downloaded, a message is placed on a channel
/// using the provided sender.
///
/// A thread is used instead of an async Tokio future because the library used
/// for the SFTP connection is not thread safe.
pub fn start_scanner(
    stop: Arc<AtomicBool>,
    mut sender: Sender<SftpDownload>,
    sqlite_path: String,
    sftp_source: SftpSource,
) -> thread::JoinHandle<Result<()>> {
    thread::spawn(move || {
        proctitle::set_title(format!("sftp-scanner {}", &sftp_source.name));

        let db_path = if sqlite_path.is_empty() {
            "/var/lib/cortex/cortex.db"
        } else {
            &sqlite_path
        };

        let conn_result = Connection::open(db_path);

        let conn = match conn_result {
            Ok(c) => {
                info!("Connected to SQLite database");
                c
            }
            Err(e) => {
                error!("Error connecting to SQLite database: {}", e);
                ::std::process::exit(2);
            }
        };

        let conn = Arc::new(Mutex::new(conn));

        let sftp_config = SftpConfig {
            address: sftp_source.address.clone(),
            username: sftp_source.username.clone(),
            password: sftp_source.password.clone(),
            key_file: sftp_source.key_file.clone(),
            compress: false,
        };

        let mut session = sftp_config
            .connect_loop(stop.clone())
            .map_err(|e| anyhow!("SFTP connect failed: {}", e))?;

        let mut sftp = session
            .sftp()
            .map_err(|e| anyhow!("SFTP connect failed: {}", e))?;

        let scan_interval = time::Duration::from_millis(sftp_source.scan_interval);
        let mut next_scan = time::Instant::now();

        while !stop.load(Ordering::Relaxed) {
            if time::Instant::now() > next_scan {
                // Increase next_scan until it is past now, because it can
                // happen that the process has stalled on SFTP reconnect,
                // causing a number of scheduled scan misses.
                while next_scan < time::Instant::now() {
                    next_scan += scan_interval;
                }

                let scan_start = time::Instant::now();
                info!("Started scanning {}", &sftp_source.name);

                let scan_result = retry(Fixed::from_millis(1000), || {
                    match scan_source(&stop, &sftp_source, &sftp, &conn, &mut sender) {
                        Ok(v) => OperationResult::Ok(v),
                        Err(e) => match e {
                            DispatcherError::DisconnectedError(_) => {
                                info!("Sftp connection disconnected, reconnecting");
                                session = match sftp_config.connect_loop(stop.clone()) {
                                    Ok(s) => s,
                                    Err(e) => {
                                        return OperationResult::Err(
                                            DispatcherError::ConnectionInterrupted(e.to_string()),
                                        )
                                    }
                                };

                                sftp = match session.sftp() {
                                    Ok(s) => s,
                                    Err(e) => {
                                        return OperationResult::Err(
                                            DispatcherError::ConnectionError(format!(
                                                "SFTP connect failed: {}",
                                                e
                                            )),
                                        )
                                    }
                                };

                                info!("Sftp connection reconnected");
                                OperationResult::Retry(e)
                            }
                            _ => OperationResult::Err(e),
                        },
                    }
                });

                match scan_result {
                    Ok(sr) => {
                        let scan_end = time::Instant::now();

                        let scan_duration = scan_end.duration_since(scan_start);

                        info!(
                            "Finished scanning {} in {} ms - {}",
                            &sftp_source.name,
                            scan_duration.as_millis(),
                            &sr
                        );

                        metrics::DIR_SCAN_COUNTER
                            .with_label_values(&[&sftp_source.name])
                            .inc();
                        metrics::DIR_SCAN_DURATION
                            .with_label_values(&[&sftp_source.name])
                            .inc_by(scan_duration.as_millis() as u64);
                    }
                    Err(e) => {
                        error!("Error scanning {}: {}", &sftp_source.name, e);
                    }
                }
            } else {
                thread::sleep(time::Duration::from_millis(200));
            }
        }

        Ok(())
    })
}

struct ScanResult {
    /// Number of files encountered during the scan
    pub encountered_files: u64,
    /// Number of files that matched the criteria of the source
    pub matching_files: u64,
    /// Number of files dispatched on the channel
    pub dispatched_files: u64,
}

impl ScanResult {
    fn new() -> ScanResult {
        ScanResult {
            encountered_files: 0,
            matching_files: 0,
            dispatched_files: 0,
        }
    }

    fn add(&mut self, other: &ScanResult) {
        self.encountered_files += other.encountered_files;
        self.matching_files += other.encountered_files;
        self.dispatched_files += other.dispatched_files;
    }
}

impl fmt::Display for ScanResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "encountered: {}, matching: {}, dispatched: {}",
            self.encountered_files, self.matching_files, self.dispatched_files
        )
    }
}

fn scan_source(
    stop: &Arc<AtomicBool>,
    sftp_source: &SftpSource,
    sftp: &ssh2::Sftp,
    conn: &Arc<Mutex<Connection>>,
    sender: &mut Sender<SftpDownload>,
) -> Result<ScanResult, DispatcherError> {
    scan_directory(
        stop,
        sftp_source,
        Path::new(&sftp_source.directory),
        sftp,
        conn,
        sender,
    )
}

fn scan_directory(
    stop: &Arc<AtomicBool>,
    sftp_source: &SftpSource,
    directory: &Path,
    sftp: &ssh2::Sftp,
    conn: &Arc<Mutex<Connection>>,
    sender: &mut Sender<SftpDownload>,
) -> Result<ScanResult, DispatcherError> {
    debug!(
        "Directory scan started for {}",
        &directory.to_str().unwrap()
    );
    let mut scan_result = ScanResult::new();

    let read_result = sftp.readdir(directory);

    let paths = match read_result {
        Ok(paths) => paths,
        Err(e) => match e.code() {
            ssh2::ErrorCode::Session(_) => {
                return Err(DispatcherError::DisconnectedError(format!(
                    "SFTP connection failed: {}",
                    e
                )))
            }
            _ => {
                return Err(DispatcherError::FileError(format!(
                    "Could not read directory: {}",
                    e
                )))
            }
        },
    };

    for (path, stat) in paths {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let file_name = path.file_name().unwrap().to_str().unwrap();

        if stat.is_dir() && sftp_source.recurse {
            let mut dir = PathBuf::from(directory);
            dir.push(file_name);
            let result = scan_directory(stop, sftp_source, &dir, sftp, conn, sender);

            match result {
                Ok(sr) => {
                    scan_result.add(&sr);
                }
                Err(e) => {
                    if let DispatcherError::DisconnectedError(_) = e {
                        return Err(e);
                    }
                }
            }
        } else {
            scan_result.encountered_files += 1;

            let file_size: u64 = stat.size.unwrap();

            let cast_result = i64::try_from(file_size);

            let file_size_db: i64 = match cast_result {
                Ok(size) => size,
                Err(e) => {
                    error!(
                        "Could not convert file size to type that can be stored in database: {}",
                        e
                    );
                    continue;
                }
            };

            let path_str = path.to_str().unwrap().to_string();

            if sftp_source.regex.is_match(file_name) {
                scan_result.matching_files += 1;
                debug!("'{}' - matches", path_str);

                let file_requires_download = if sftp_source.deduplicate {
                    let conn = conn.lock().unwrap();
                    let mut stmt = conn
                        .prepare(
                            "select count(*) from sftp_download where source = ?1 and path = ?2 and size = ?3",
                        )
                        .map_err(|e| {
                            DispatcherError::DatabaseError(format!(
                                "Error preparing query: {}",
                                e
                            ))
                        })?;
                    let query_result = stmt.query_row(
                        params![&sftp_source.name, &path_str, &file_size_db],
                        |row| row.get::<_, i64>(0),
                    );

                    match query_result {
                        Ok(count) => count == 0,
                        Err(e) => {
                            return Err(DispatcherError::DatabaseError(format!(
                                "Error querying database: {}",
                                e
                            )));
                        }
                    }
                } else {
                    true
                };

                if file_requires_download {
                    let mut conn = conn.lock().unwrap();
                    let tx = conn.transaction().map_err(|e| {
                        DispatcherError::DatabaseError(format!("Error starting transaction: {}", e))
                    })?;
                    let insert_result = tx.execute(
                        "insert into sftp_download (source, path, size) values (?1, ?2, ?3)",
                        params![&sftp_source.name, &path_str, &file_size_db],
                    );

                    let sftp_download_id = match insert_result {
                        Ok(_) => tx.last_insert_rowid(),
                        Err(e) => {
                            return Err(DispatcherError::DatabaseError(format!(
                                "Error inserting record: {}",
                                e
                            )));
                        }
                    };
                    tx.commit().map_err(|e| {
                        DispatcherError::DatabaseError(format!(
                            "Error committing transaction: {}",
                            e
                        ))
                    })?;

                    let command = SftpDownload {
                        id: sftp_download_id,
                        created: Utc::now(),
                        size: stat.size,
                        sftp_source: sftp_source.name.clone(),
                        path: path_str.clone(),
                        remove: sftp_source.remove,
                    };

                    let retry_policy = Fixed::from_millis(100);
                    let send_timeout = time::Duration::from_millis(1000);

                    let send_result = retry(retry_policy, || {
                        let result = sender.send_timeout(command.clone(), send_timeout);

                        match result {
                            Ok(()) => {
                                scan_result.dispatched_files += 1;
                                debug!("Sent message {} on channel", command);
                                OperationResult::Ok(())
                            }
                            Err(e) => match e {
                                SendTimeoutError::Timeout(timeout) => {
                                    OperationResult::Retry(timeout)
                                }
                                SendTimeoutError::Disconnected(timeout) => {
                                    OperationResult::Err(timeout)
                                }
                            },
                        }
                    });

                    match send_result {
                        Ok(_) => (),
                        Err(e) => error!("Error sending download message on channel: {:?}", e),
                    }
                } else {
                    debug!("{} already encountered {}", sftp_source.name, path_str);
                }
            } else {
                debug!(" - {} - no match", path_str);
            }
        }
    }

    Ok(scan_result)
}
