#[cfg(test)]
mod tests {
    use std::io::Write;

    use assert_cmd::cmd::Command;

    use dev_stack::dev_stack::DevStack;

    fn render_cortex_config(rabbitmq_host: url::Host, rabbitmq_port: u16) -> String {
        format!(
            r###"
storage:
  directory: /home/alfred/projects/cortex-dispatcher/dev-stack/tmp/storage

command_queue:
  address: "amqp://{rabbitmq_host}:{rabbitmq_port}/%2f"

directory_sources:
  - name: mixed-directory
    directory: /home/alfred/projects/cortex-dispatcher/dev-stack/tmp/incoming
    recursive: True
    events:
      - CloseWrite
      - MovedTo
    filter:
      Regex:
        pattern: ".*\\.txt$"

scan_interval: 5000

directory_targets:
  - name: v5
    directory: /home/alfred/projects/cortex-dispatcher/dev-stack/tmp/storage/v5
    overwrite: false
    permissions: 0o644
  - name: v6
    directory: /home/alfred/projects/cortex-dispatcher/dev-stack/tmp/storage/v6
    overwrite: false
    permissions: 0o644
  - name: red
    directory: /home/alfred/projects/cortex-dispatcher/dev-stack/tmp/storage/red-consumer
    overwrite: false
    permissions: 0o644
    notify:
      rabbitmq:
        message_template: '{{"type": "new_file", "file_path": "{{ file_path }}"}}'
        address: "amqp://127.0.0.1:5672/%2f"
        exchange: ""
        routing_key: "processing-node-red"
  - name: blue
    directory: /home/alfred/projects/cortex-dispatcher/dev-stack/tmp/storage/blue-consumer
    overwrite: false
    permissions: 0o644
    notify:
      rabbitmq:
        message_template: '{{"type": "new_file", "file_path": "{{ file_path }}"}}'
        address: "amqp://127.0.0.1:5672/%2f"
        exchange: ""
        routing_key: "processing-node-blue"

sftp_sources: []

connections:
  - source: mixed-directory
    target: v5
    filter:
      Regex:
        pattern: "^.*-v5\\.xml$"
  - source: mixed-directory
    target: v6
    filter:
      Regex:
        pattern: "^.*-v6\\.xml$"
  - source: local-red
    target: red
  - source: local-blue
    target: blue

sqlite:
  path: /tmp/cortex-test.db

http_server:
  address: "0.0.0.0:56008"
"###
        )
    }

    #[tokio::test]
    async fn start_cortex_dispatcher() -> Result<(), Box<dyn std::error::Error>> {
        let dev_stack = DevStack::start(true).await.unwrap();

        let mut cortex_config_file = tempfile::NamedTempFile::new().unwrap();

        let cortex_config = render_cortex_config(
            dev_stack.rabbitmq_host().await.unwrap(),
            dev_stack.rabbitmq_port().await.unwrap(),
        );

        cortex_config_file
            .write_all(cortex_config.as_bytes())
            .unwrap();

        let current_dir = std::env::current_dir();
        let target_dir = current_dir
            .as_ref()
            .unwrap()
            .parent()
            .unwrap()
            .join("target")
            .join("debug");
        
        let mut cmd = Command::new(target_dir.join("cortex-dispatcher"));

        cmd.timeout(std::time::Duration::from_secs(5));
        cmd.env("RUST_LOG", "debug");
 
        cmd.arg("service")
            .arg("--config")
            .arg(cortex_config_file.path());

        cmd.assert()
            .stderr(predicates::prelude::predicate::str::contains(
                "Configuration loaded",
            ));

        Ok(())
    }
}
