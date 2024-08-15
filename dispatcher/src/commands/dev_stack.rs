use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use clap::Parser;
use cortex_core::create_schema;
use tokio::signal;

use crate::commands::{Cmd, CmdResult};

use dev_stack::dev_stack::DevStack;

#[derive(Parser, Debug)]
pub struct DevStackOpt {
    #[arg(short, long, help = "Start data generator")]
    data_generator: bool,
}

impl Cmd for DevStackOpt {
    fn run(&self) -> CmdResult {
        let mut env_logger_builder = env_logger::builder();

        env_logger_builder
            .format(|buf, record| writeln!(buf, "{}  {}", record.level(), record.args()));

        env_logger_builder.init();

        let rt = tokio::runtime::Runtime::new().unwrap();

        println!("Starting development stack");

        rt.block_on(start_dev_stack(self.data_generator));

        println!("Done");

        Ok(())
    }
}

async fn start_dev_stack(data_generator: bool) {
    let postgres_config_file =
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/postgresql.conf"));

    let dev_stack = DevStack::start(&postgres_config_file, false).await.unwrap();

    let database = dev_stack.test_database().await.unwrap();

    let mut client = database.connect().await.unwrap();

    create_schema(&mut client).await.unwrap();

    let tmp_dir = "tmp";

    let data_dir: PathBuf = [tmp_dir, "incoming"].iter().collect();

    std::fs::create_dir_all(&data_dir).unwrap();

    if data_generator {
        println!("Starting data generator");
        tokio::spawn(generate_data(data_dir.clone()));
        println!("Data generator is running");
    }

    let cortex_config_file_name: &str = "cortex-dispatcher.yml";

    let mut cortex_config_file_path = PathBuf::new();
    cortex_config_file_path.push(tmp_dir);
    cortex_config_file_path.push(cortex_config_file_name);

    let mut cortex_config_file = std::fs::File::create(&cortex_config_file_path).unwrap();

    let postgres_host = dev_stack.postgres_host().await.unwrap();
    let postgres_port = dev_stack.postgres_port().await.unwrap();
    let database_name = database.name;
    let rabbitmq_host = dev_stack.rabbitmq_host().await.unwrap();
    let rabbitmq_port = dev_stack.rabbitmq_port().await.unwrap();

    let cortex_config = render_cortex_config(
        postgres_host.clone(),
        postgres_port,
        &database_name,
        rabbitmq_host.clone(),
        rabbitmq_port,
        tmp_dir,
    );

    cortex_config_file
        .write_all(cortex_config.as_bytes())
        .unwrap();

    println!();
    println!(
        "PostgreSQL available at: {}:{}",
        postgres_host, postgres_port
    );
    println!(
        "RabbitMQ available at:   {}:{}",
        rabbitmq_host, rabbitmq_port
    );
    println!();
    println!(
        "Cortex Dispatcher config file available at: '{}'",
        cortex_config_file_path.to_string_lossy()
    );

    signal::ctrl_c().await.unwrap();

    println!("Stopping development stack");
}

async fn generate_data<S: AsRef<Path>>(data_dir: S) {
    loop {
        let timestamp = chrono::Utc::now();
        let file_name = format!("test_file_{}_v5.csv", timestamp.format("%Y%m%d_%H%M%S"));

        let mut file_path = PathBuf::new();
        file_path.push(&data_dir);
        file_path.push(file_name);

        generate_file(&file_path);

        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    }
}

fn generate_file(file_path: &Path) {
    let data_file = File::create(file_path).unwrap();

    let mut buf_writer = BufWriter::new(data_file);

    for i in 0..100 {
        buf_writer
            .write_fmt(format_args!("This is line {}\n", i))
            .unwrap();
    }
}

fn render_cortex_config(
    postgres_host: url::Host,
    postgres_port: u16,
    database: &str,
    rabbitmq_host: url::Host,
    rabbitmq_port: u16,
    root_dir: &str,
) -> String {
    format!(
        r###"
storage:
  directory: {root_dir}/storage

command_queue:
  address: "amqp://{rabbitmq_host}:{rabbitmq_port}/%2f"

directory_sources:
- name: mixed-directory
  directory: {root_dir}/incoming
  recursive: True
  events:
  - CloseWrite
  - MovedTo
  filter:
    Regex:
      pattern: ".*\\.csv$"

scan_interval: 5000

directory_targets:
- name: v5
  directory: {root_dir}/storage/v5
  overwrite: false
  permissions: 0o644
- name: v6
  directory: {root_dir}/storage/v6
  overwrite: false
  permissions: 0o644
- name: red
  directory: {root_dir}/storage/red-consumer
  overwrite: false
  permissions: 0o644
  notify:
    rabbitmq:
      message_template: '{{"type": "new_file", "file_path": "{{ file_path }}"}}'
      address: "amqp://127.0.0.1:5672/%2f"
      exchange: ""
      routing_key: "processing-node-red"
- name: blue
  directory: {root_dir}/storage/blue-consumer
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
      pattern: "^.*-v5\\.csv$"
- source: mixed-directory
  target: v6
  filter:
    Regex:
      pattern: "^.*-v6\\.csv$"
- source: local-red
  target: red
- source: local-blue
  target: blue

postgresql:
  url: "postgresql://postgres@{postgres_host}:{postgres_port}/{database}"

http_server:
  address: "0.0.0.0:56008"
"###
    )
}
