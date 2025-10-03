use std::convert::TryFrom;
use std::fs::{rename, File};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{thread, time};

use crossbeam_channel::{Receiver, RecvTimeoutError};
use log::{debug, error, info};

use retry::{delay::Fixed, retry, OperationResult};

use anyhow::Result;

use crate::base_types::MessageResponse;
use crate::event::FileEvent;
use crate::local_storage::LocalStorage;
use crate::metrics;
use crate::persistence::Persistence;
use crate::settings;

use cortex_core::error::DispatcherError;
use cortex_core::sftp_connection::SftpConfig;
use cortex_core::SftpDownload;

use sha2::{Digest, Sha256};
use tee::TeeReader;

use chrono::{DateTime, Utc};

pub struct SftpDownloader<T>
where
    T: Persistence,
{
    pub sftp_source: settings::SftpSource,
    pub persistence: T,
    pub local_storage: LocalStorage<T>,
}

impl<T> SftpDownloader<T>
where
    T: Persistence,
    T: Send,
    T: Clone,
    T: 'static,
{
    pub fn start(
        stop: Arc<AtomicBool>,
        receiver: Receiver<(u64, SftpDownload)>,
        ack_sender: async_channel::Sender<MessageResponse>,
        config: settings::SftpSource,
        sender: tokio::sync::mpsc::UnboundedSender<FileEvent>,
        local_storage: LocalStorage<T>,
        persistence: T,
    ) -> thread::JoinHandle<Result<(), DispatcherError>> {
        thread::spawn(move || -> Result<(), DispatcherError> {
            proctitle::set_title("sftp_dl");

            let sftp_config = SftpConfig {
                address: config.address.clone(),
                username: config.username.clone(),
                password: config.password.clone(),
                key_file: config.key_file.clone(),
                compress: config.compress,
            };

            let mut session = sftp_config
                .connect_loop(stop.clone())
                .map_err(|e| DispatcherError::ConnectionError(e.to_string()))?;

            let mut sftp = session
                .sftp()
                .map_err(|e| DispatcherError::ConnectionError(e.to_string()))?;

            let mut sftp_downloader = SftpDownloader {
                sftp_source: config.clone(),
                persistence,
                local_storage: local_storage.clone(),
            };

            let timeout = time::Duration::from_millis(500);

            // Take SFTP download commands from the queue until the stop flag is set and
            // the command channel is empty.
            while !(stop.load(Ordering::Relaxed) && receiver.is_empty()) {
                let receive_result = receiver.recv_timeout(timeout);

                match receive_result {
                    Ok((_delivery_tag, command)) => {
                        let download_result = retry(Fixed::from_millis(1000), || {
                            match sftp_downloader.handle(&sftp, &command) {
                                Ok(file_event) => OperationResult::Ok(file_event),
                                Err(e) => match e {
                                    DispatcherError::DisconnectedError(_) => {
                                        info!("Sftp connection disconnected, reconnecting");
                                        session = match sftp_config.connect_loop(stop.clone()) {
                                            Ok(s) => s,
                                            Err(e) => {
                                                return OperationResult::Err(
                                                    DispatcherError::ConnectionInterrupted(
                                                        e.to_string(),
                                                    ),
                                                )
                                            }
                                        };

                                        sftp = match session.sftp() {
                                            Ok(s) => s,
                                            Err(e) => {
                                                return OperationResult::Err(
                                                    DispatcherError::ConnectionError(e.to_string()),
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

                        match download_result {
                            Ok(file_event) => {
                                let send_result = ack_sender.try_send(MessageResponse::Ack {});

                                match send_result {
                                    Ok(_) => {
                                        debug!("Sent message ack to channel");
                                    }
                                    Err(e) => {
                                        error!("Error sending message ack to channel: {}", e);
                                    }
                                }

                                if let Some(f) = file_event {
                                    // Notify about new data from this SFTP source
                                    let send_result = sender.send(f);

                                    match send_result {
                                        Ok(_) => {
                                            debug!("Sent SFTP FileEvent to channel");
                                        }
                                        Err(e) => {
                                            error!("Error notifying consumers of new file: {}", e);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                let send_result = ack_sender.try_send(MessageResponse::Nack {});

                                match send_result {
                                    Ok(_) => {
                                        debug!("Sent message nack to channel");
                                    }
                                    Err(e) => {
                                        error!("Error sending message nack to channel: {}", e);
                                    }
                                }

                                error!("[E01003] Error downloading '{}': {}", &command.path, e);
                            }
                        }
                    }
                    Err(e) => {
                        match e {
                            RecvTimeoutError::Timeout => (),
                            RecvTimeoutError::Disconnected => {
                                // If the stop flag was set, the other side of the channel was
                                // dropped because of that, otherwise return an error
                                if stop.load(Ordering::Relaxed) {
                                    return Ok(());
                                } else {
                                    error!("[E02005] SFTP download command channel receiver disconnected");

                                    return Err(DispatcherError::DisconnectedError(format!(
                                        "SFTP download command channel receiver disconnected: {}",
                                        e
                                    )));
                                }
                            }
                        }
                    }
                }
            }

            debug!("SFTP source stream '{}' ended", config.name);

            Ok(())
        })
    }

    pub fn handle(
        &mut self,
        sftp: &ssh2::Sftp,
        msg: &SftpDownload,
    ) -> Result<Option<FileEvent>, DispatcherError> {
        let remote_path = Path::new(&msg.path);

        let path_prefix = Path::new("");

        let local_path = self
            .local_storage
            .local_path(&self.sftp_source.name, &remote_path, &Path::new("/"))
            .map_err(|e| DispatcherError::FileError(format!("Could not localize path: {}", e)))?;

        match msg.size {
            Some(size) => {
                debug!(
                    "Downloading <{}> '{}' -> '{}' {} bytes",
                    self.sftp_source.name,
                    msg.path,
                    local_path.to_string_lossy(),
                    size
                );
            }
            None => {
                debug!(
                    "Downloading <{}> '{}' size unknown",
                    self.sftp_source.name, msg.path
                );
            }
        }

        let mut remote_file = sftp.open(remote_path).map_err(|e| {
            match e.code() {
                ssh2::ErrorCode::Session(_) => {
                    // Probably a fault in the SFTP connection
                    DispatcherError::DisconnectedError(e.to_string())
                }
                ssh2::ErrorCode::SFTP(2) => {
                    let delete_result = self.persistence.delete_sftp_download_file(msg.id);

                    match delete_result {
                        Ok(_) => DispatcherError::NoSuchFile,
                        Err(e) => DispatcherError::PersistenceError(format!(
                            "Error removing record of non-existent remote file: {}",
                            e
                        )),
                    }
                }
                _ => DispatcherError::FileError(format!("Error opening remote file: {}", e)),
            }
        })?;

        let stat = remote_file.stat().map_err(|e| match e.code() {
            ssh2::ErrorCode::Session(_) => {
                // Probably a fault in the SFTP connection
                DispatcherError::DisconnectedError(e.to_string())
            }
            _ => {
                DispatcherError::FileError(format!("Error retrieving stat for remote file: {}", e))
            }
        })?;

        let mtime = stat.mtime.unwrap_or(0);

        let sec = i64::try_from(mtime).map_err(|e| {
            DispatcherError::OtherError(format!("Error converting mtime to i64: {}", e))
        })?;
        let nsec = 0;

        let modified: DateTime<Utc> = DateTime::from_timestamp(sec, nsec).unwrap();

        let file_info_result = self
            .local_storage
            .get_file_info(&msg.sftp_source, &remote_path, &path_prefix)
            .map_err(|e| {
                DispatcherError::OtherError(format!(
                    "Could not get file information from internal storage: {}",
                    e
                ))
            })?;

        // Opportunity for duplicate check without hash check
        if let Some(file_info) = &file_info_result {
            // See if a deduplication check is configured
            if let settings::Deduplication::Check(check) = &self.sftp_source.deduplication {
                // Only check now if no hash check is required, because that is not calculated
                // yet
                if !check.hash && check.equal(file_info, stat.size.unwrap(), modified, None) {
                    // A file with the same name, modified timestamp and/or size was already
                    // downloaded, so assume that it is the same and skip.
                    return Ok(None);
                }
            }
        }

        if let Some(local_path_parent) = local_path.parent() {
            if !local_path_parent.exists() {
                std::fs::create_dir_all(local_path_parent).map_err(|e| {
                    DispatcherError::OtherError(format!(
                        "Error creating containing directory '{}': {}",
                        local_path_parent.to_string_lossy(),
                        e
                    ))
                })?;

                info!(
                    "Created containing directory '{}'",
                    local_path_parent.to_string_lossy()
                );
            }
        }

        // Construct a temporary file name with the extension '.part'
        let mut local_path_part = local_path.as_os_str().to_os_string();
        local_path_part.push(".part");

        let mut local_file_part = File::create(&local_path_part).map_err(|e| {
            DispatcherError::FileError(format!(
                "Error creating local file part '{}': {}",
                local_path.to_string_lossy(),
                e
            ))
        })?;

        let mut sha256 = Sha256::new();

        let mut tee_reader = TeeReader::new(&mut remote_file, &mut sha256);

        let copy_result = io::copy(&mut tee_reader, &mut local_file_part);
        let hash = format!("{:x}", sha256.finalize());

        if let Some(file_info) = &file_info_result {
            // See if a deduplication check is configured
            if let settings::Deduplication::Check(check) = &self.sftp_source.deduplication {
                if check.equal(file_info, stat.size.unwrap(), modified, Some(hash.clone())) {
                    // A file with the same name, modified timestamp, size and/or hash was already
                    // downloaded, so assume that it is the same and skip.
                    return Ok(None);
                }
            }
        }

        let bytes_copied = copy_result
            .map_err(|e| DispatcherError::OtherError(format!("Error copying file: {}", e)))?;

        info!(
            "Downloaded <{}> '{}' {} bytes",
            self.sftp_source.name, msg.path, bytes_copied
        );

        // Rename the file to its regular name
        rename(&local_path_part, &local_path).map_err(|e| {
            DispatcherError::OtherError(format!("Error renaming part to its regular name: {}", e))
        })?;

        let file_size = i64::try_from(bytes_copied).map_err(|e| {
            DispatcherError::OtherError(format!("Error converting bytes copied to i64: {}", e))
        })?;

        let file_id = self
            .persistence
            .insert_file(
                &self.sftp_source.name,
                &local_path.to_string_lossy(),
                &modified,
                file_size,
                Some(hash.clone()),
            )
            .map_err(|_| {
                DispatcherError::PersistenceError(
                    "Error inserting file into persistence".to_string(),
                )
            })?;

        self.persistence
            .set_sftp_download_file(msg.id, file_id)
            .map_err(|e| {
                DispatcherError::OtherError(format!(
                    "Error updating SFTP download information: {}",
                    e
                ))
            })?;

        metrics::FILE_DOWNLOAD_COUNTER_VEC
            .with_label_values(&[&self.sftp_source.name])
            .inc();
        metrics::BYTES_DOWNLOADED_COUNTER_VEC
            .with_label_values(&[&self.sftp_source.name])
            .inc_by(bytes_copied);

        if msg.remove {
            let unlink_result = sftp.unlink(remote_path);

            match unlink_result {
                Ok(_) => {
                    debug!("Removed <{}> '{}'", self.sftp_source.name, msg.path);
                }
                Err(e) => {
                    error!(
                        "Error removing <{}> '{}': {}",
                        self.sftp_source.name, msg.path, e
                    );
                }
            }
        }

        Ok(Some(FileEvent {
            file_id,
            source_name: self.sftp_source.name.clone(),
            path: local_path,
            hash,
        }))
    }
}
