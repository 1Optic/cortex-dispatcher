CREATE INDEX IF NOT EXISTS dispatched_file_id_idx ON dispatched(file_id);
CREATE INDEX IF NOT EXISTS directory_source_file_id_idx ON directory_source(file_id);
CREATE INDEX IF NOT EXISTS sftp_download_file_id_idx ON sftp_download(file_id);