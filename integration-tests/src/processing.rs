#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

    use dev_stack::dev_stack::DevStack;
    use flate2::{Compression, GzBuilder};
    use rusqlite::Connection;
    use tempfile::{NamedTempFile, TempDir};

    struct RunningService {
        _dev_stack: DevStack,
        _temp_dir: TempDir,
        _config_file: NamedTempFile,
        child: Child,
    }

    impl Drop for RunningService {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    struct TestDirectories {
        root: PathBuf,
        incoming: PathBuf,
        storage: PathBuf,
    }

    impl TestDirectories {
        fn new() -> (Self, TempDir) {
            let temp_dir = tempfile::tempdir().unwrap();
            let root = temp_dir.path().to_path_buf();
            let incoming = root.join("incoming");
            let storage = root.join("storage");
            fs::create_dir_all(&incoming).unwrap();
            fs::create_dir_all(&storage).unwrap();
            (
                Self {
                    root,
                    incoming,
                    storage,
                },
                temp_dir,
            )
        }
    }

    fn binary_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("target/debug/cortex-dispatcher")
    }

    async fn start_service(
        directories: TestDirectories,
        temp_dir: TempDir,
        source_config: &str,
        connections: &str,
        scan_interval: u64,
    ) -> RunningService {
        let dev_stack = DevStack::start(false).await.unwrap();
        let config = format!(
            r#"
storage:
  directory: {}
command_queue:
  address: "amqp://{}:{}/%2f"
directory_sources:
{}
scan_interval: {}
directory_targets:
  - name: output
    directory: {}
    overwrite: false
    permissions: 0o644
sftp_sources: []
connections:
{}
sqlite:
  path: {}
http_server:
  address: "127.0.0.1:0"
"#,
            directories.root.join("dispatcher-storage").display(),
            dev_stack.rabbitmq_host().await.unwrap(),
            dev_stack.rabbitmq_port().await.unwrap(),
            source_config,
            scan_interval,
            directories.storage.display(),
            connections,
            directories.root.join("cortex.db").display(),
        );

        let mut config_file = NamedTempFile::new().unwrap();
        config_file.write_all(config.as_bytes()).unwrap();

        let child = Command::new(binary_path())
            .arg("service")
            .arg("--config")
            .arg(config_file.path())
            .env("RUST_LOG", "debug")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        wait_for_file(&directories.root.join("cortex.db")).await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        RunningService {
            _dev_stack: dev_stack,
            _temp_dir: temp_dir,
            _config_file: config_file,
            child,
        }
    }

    async fn wait_for_file(path: &Path) {
        for _ in 0..50 {
            if path.is_file() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("timed out waiting for {}", path.display());
    }

    async fn wait_for_missing(path: &Path) {
        for _ in 0..50 {
            if !path.exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("timed out waiting for {} to disappear", path.display());
    }

    async fn wait_for_dispatches(path: &Path, expected: i64) {
        for _ in 0..50 {
            if let Ok(connection) = Connection::open(path)
                && let Ok(count) = connection.query_row(
                    "select count(*) from dispatched",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                && count == expected
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("timed out waiting for {expected} dispatches in {}", path.display());
    }

    fn source(name: &str, directory: &Path, extra: &str) -> String {
        format!(
            "  - name: {name}\n    directory: {}\n    events:\n      - CloseWrite\n{extra}",
            directory.display()
        )
    }

    fn connection(source: &str) -> String {
        format!("  - source: {source}\n    target: output\n")
    }

    fn write_file(path: &Path, contents: &[u8]) {
        let mut file = File::create(path).unwrap();
        file.write_all(contents).unwrap();
    }

    #[tokio::test]
    async fn deduplicates_same_file_by_hash() {
        let (directories, temp_dir) = TestDirectories::new();
        let source_config = source("hash-source", &directories.incoming, "");
        let connections = connection("hash-source");
        let service = start_service(directories, temp_dir, &source_config, &connections, 100).await;
        let input = service._temp_dir.path().join("incoming/file.txt");
        let target = service._temp_dir.path().join("storage/file.txt");

        write_file(&input, b"same content");
        wait_for_file(&target).await;
        wait_for_missing(&input).await;
        let database = service._temp_dir.path().join("cortex.db");
        wait_for_dispatches(&database, 1).await;

        write_file(&input, b"same content");
        wait_for_missing(&input).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let count = Connection::open(database)
            .unwrap()
            .query_row("select count(*) from dispatched", [], |row| row.get::<_, i64>(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn deduplicates_same_file_by_name() {
        let (directories, temp_dir) = TestDirectories::new();
        let source_config = source(
            "name-source",
            &directories.incoming,
            "    deduplication: name\n",
        );
        let connections = connection("name-source");
        let service = start_service(directories, temp_dir, &source_config, &connections, 100).await;
        let input = service._temp_dir.path().join("incoming/file.txt");
        let target = service._temp_dir.path().join("storage/file.txt");

        write_file(&input, b"first content");
        wait_for_file(&target).await;
        wait_for_missing(&input).await;
        let first_contents = fs::read(&target).unwrap();
        let database = service._temp_dir.path().join("cortex.db");
        wait_for_dispatches(&database, 1).await;

        write_file(&input, b"different content");
        wait_for_missing(&input).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(first_contents, fs::read(&target).unwrap());
        let count = Connection::open(database)
            .unwrap()
            .query_row("select count(*) from dispatched", [], |row| row.get::<_, i64>(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn honors_delete_setting() {
        let (directories, temp_dir) = TestDirectories::new();
        let retained = directories.root.join("retained");
        let deleted = directories.root.join("deleted");
        fs::create_dir_all(&retained).unwrap();
        fs::create_dir_all(&deleted).unwrap();
        let source_config = format!(
            "{}{}",
            source("retained", &retained, "    delete: false\n"),
            source("deleted", &deleted, "    delete: true\n")
        );
        let connections = format!("{}{}", connection("retained"), connection("deleted"));
        let service = start_service(directories, temp_dir, &source_config, &connections, 100).await;
        let retained_file = service._temp_dir.path().join("retained/kept.txt");
        let deleted_file = service._temp_dir.path().join("deleted/removed.txt");
        write_file(&retained_file, b"keep");
        write_file(&deleted_file, b"remove");
        wait_for_file(&service._temp_dir.path().join("storage/kept.txt")).await;
        wait_for_file(&service._temp_dir.path().join("storage/removed.txt")).await;
        assert!(retained_file.exists());
        wait_for_missing(&deleted_file).await;
    }

    #[tokio::test]
    async fn honors_recursive_and_new_directory_watches() {
        let (directories, temp_dir) = TestDirectories::new();
        let flat = directories.root.join("flat");
        let recursive = directories.root.join("recursive");
        fs::create_dir_all(flat.join("existing")).unwrap();
        fs::create_dir_all(recursive.join("existing")).unwrap();
        let source_config = format!(
            "{}{}",
            source("flat", &flat, "    recursive: false\n"),
            source("recursive", &recursive, "    recursive: true\n")
        );
        let connections = format!("{}{}", connection("flat"), connection("recursive"));
        let service = start_service(directories, temp_dir, &source_config, &connections, 100).await;
        write_file(&flat.join("root.txt"), b"root");
        write_file(&flat.join("existing/flat-nested.txt"), b"ignored");
        write_file(&recursive.join("existing/nested.txt"), b"existing");
        wait_for_file(&service._temp_dir.path().join("storage/root.txt")).await;
        wait_for_file(&service._temp_dir.path().join("storage/nested.txt")).await;
        assert!(!service._temp_dir.path().join("storage/flat-nested.txt").exists());

        let new_directory = recursive.join("created");
        fs::create_dir(&new_directory).unwrap();
        write_file(&new_directory.join("new.txt"), b"created");
        wait_for_file(&service._temp_dir.path().join("storage/new.txt")).await;
    }

    #[tokio::test]
    async fn applies_regex_filter() {
        let (directories, temp_dir) = TestDirectories::new();
        let source_config = source(
            "filtered",
            &directories.incoming,
            "    filter:\n      Regex:\n        pattern: \".*\\\\.txt$\"\n",
        );
        let connections = connection("filtered");
        let service = start_service(directories, temp_dir, &source_config, &connections, 100).await;
        let accepted = service._temp_dir.path().join("incoming/accepted.txt");
        let rejected = service._temp_dir.path().join("incoming/rejected.csv");
        write_file(&accepted, b"accepted");
        write_file(&rejected, b"rejected");
        wait_for_file(&service._temp_dir.path().join("storage/accepted.txt")).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(rejected.exists());
        assert!(!service._temp_dir.path().join("storage/rejected.csv").exists());
    }

    #[tokio::test]
    async fn handles_unpack_before_hash_and_close_write_and_moved_to() {
        let (directories, temp_dir) = TestDirectories::new();
        let source_config = format!(
            "  - name: events\n    directory: {}\n    unpack_before_hash: true\n    events:\n      - CloseWrite\n      - MovedTo\n",
            directories.incoming.display()
        );
        let connections = connection("events");
        let service = start_service(directories, temp_dir, &source_config, &connections, 100).await;
        let input = service._temp_dir.path().join("incoming/archive.gz");
        let staged = service._temp_dir.path().join("staged.gz");

        let mut encoder = GzBuilder::new().mtime(1).write(File::create(&input).unwrap(), Compression::default());
        encoder.write_all(b"unpacked content").unwrap();
        encoder.finish().unwrap();
        let target = service._temp_dir.path().join("storage/archive.gz");
        wait_for_file(&target).await;
        wait_for_missing(&input).await;
        let first_contents = fs::read(&target).unwrap();

        let mut encoder = GzBuilder::new().mtime(2).write(File::create(&staged).unwrap(), Compression::best());
        encoder.write_all(b"unpacked content").unwrap();
        encoder.finish().unwrap();
        fs::rename(&staged, &input).unwrap();
        wait_for_missing(&input).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(first_contents, fs::read(&target).unwrap());
        let count = Connection::open(service._temp_dir.path().join("cortex.db"))
            .unwrap()
            .query_row("select count(*) from dispatched", [], |row| row.get::<_, i64>(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}