-- SQLite initial schema for Cortex Dispatcher (minimal equivalent)

-- Files registered in internal storage
CREATE TABLE IF NOT EXISTS file (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  timestamp TEXT NOT NULL DEFAULT (datetime('now')),
  source TEXT NOT NULL,
  path TEXT NOT NULL,
  modified TEXT NOT NULL,
  size INTEGER NOT NULL,
  hash TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS file_index ON file (source, path);

-- Records of SFTP downloads to perform
CREATE TABLE IF NOT EXISTS sftp_download (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  timestamp TEXT NOT NULL DEFAULT (datetime('now')),
  source TEXT NOT NULL,
  path TEXT NOT NULL,
  size INTEGER,
  file_id INTEGER,
  FOREIGN KEY (file_id) REFERENCES file(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS sftp_download_file_index ON sftp_download (source, path);

-- Local directory source tracking
CREATE TABLE IF NOT EXISTS directory_source (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  timestamp TEXT NOT NULL DEFAULT (datetime('now')),
  source TEXT NOT NULL,
  path TEXT NOT NULL,
  modified TEXT NOT NULL,
  size INTEGER NOT NULL,
  file_id INTEGER,
  FOREIGN KEY (file_id) REFERENCES file(id) ON DELETE CASCADE
);

-- Dispatched files tracking
CREATE TABLE IF NOT EXISTS dispatched (
  file_id INTEGER NOT NULL,
  target TEXT NOT NULL,
  timestamp TEXT NOT NULL DEFAULT (datetime('now')),
  FOREIGN KEY (file_id) REFERENCES file(id) ON DELETE CASCADE
);