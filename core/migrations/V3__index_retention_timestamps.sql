CREATE INDEX IF NOT EXISTS file_timestamp_idx ON file(timestamp);
CREATE INDEX IF NOT EXISTS sftp_download_timestamp_idx ON sftp_download(timestamp);
CREATE INDEX IF NOT EXISTS directory_source_timestamp_idx ON directory_source(timestamp);
CREATE INDEX IF NOT EXISTS dispatched_timestamp_idx ON dispatched(timestamp);