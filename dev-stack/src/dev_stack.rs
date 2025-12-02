use testcontainers::core::{Mount, WaitFor};
use testcontainers::{runners::AsyncRunner, ContainerAsync, ContainerRequest, ImageExt};

use rand::distr::{Alphanumeric, SampleString};
use tokio::io::AsyncBufReadExt;

use thiserror::Error;

const RABBITMQ_NAME: &str = "rabbitmq";
const RABBITMQ_TAG: &str = "3.11.9-management";

#[derive(Error, Debug)]
pub enum DevStackError {
    #[error("Container issue with dev stack: {0}")]
    Testcontainer(#[from] testcontainers::TestcontainersError),
}

pub struct DevStack {
    pub rabbitmq_container: ContainerAsync<RabbitMq>,
}

pub fn print_stdout<
    I: tokio::io::AsyncBufRead + std::marker::Unpin + std::marker::Send + 'static,
>(
    prefix: String,
    mut reader: I,
) {
    tokio::spawn(async move {
        let mut buffer = String::new();
        loop {
            let result = reader.read_line(&mut buffer).await;

            if let Ok(0) = result {
                break;
            };

            print!("{prefix} - {buffer}");

            buffer.clear();
        }
    });
}

impl DevStack {
    pub async fn start(print_output: bool) -> Result<DevStack, DevStackError> {
        let rabbitmq_name = format!("rabbitmq-{}", generate_name(8));
        let rabbitmq_container = create_rabbitmq_container(&rabbitmq_name)
            .start()
            .await
            .unwrap();

        if print_output {
            print_stdout("rabbitmq - ".to_string(), rabbitmq_container.stdout(true));
        }

        Ok(DevStack { rabbitmq_container })
    }

    pub async fn rabbitmq_host(&self) -> Result<url::Host, DevStackError> {
        self.rabbitmq_container
            .get_host()
            .await
            .map_err(DevStackError::Testcontainer)
    }

    pub async fn rabbitmq_port(&self) -> Result<u16, DevStackError> {
        self.rabbitmq_container
            .get_host_port_ipv4(5672)
            .await
            .map_err(DevStackError::Testcontainer)
    }
}

pub fn generate_name(len: usize) -> String {
    Alphanumeric.sample_string(&mut rand::rng(), len)
}

#[derive(Debug, Default, Clone)]
pub struct RabbitMq;

impl testcontainers::Image for RabbitMq {
    fn name(&self) -> &str {
        RABBITMQ_NAME
    }

    fn tag(&self) -> &str {
        RABBITMQ_TAG
    }

    fn ready_conditions(&self) -> Vec<WaitFor> {
        vec![WaitFor::message_on_stdout(
            "Server startup complete; 4 plugins started.",
        )]
    }
}

pub fn create_rabbitmq_container(name: &str) -> ContainerRequest<RabbitMq> {
    let conf_path = concat!(env!("CARGO_MANIFEST_DIR"), "/rabbitmq.conf");
    let definitions_path = concat!(env!("CARGO_MANIFEST_DIR"), "/definitions.json");

    ContainerRequest::from(RabbitMq)
        .with_container_name(name)
        .with_mount(Mount::bind_mount(conf_path, "/etc/rabbitmq/rabbitmq.conf"))
        .with_mount(Mount::bind_mount(
            definitions_path,
            "/etc/rabbitmq/definitions.json",
        ))
}
