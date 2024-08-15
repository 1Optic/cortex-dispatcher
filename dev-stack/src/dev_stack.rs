use std::path::Path;
use testcontainers::core::{ContainerPort, Mount, WaitFor};
use testcontainers::{
    runners::AsyncRunner, ContainerAsync, ContainerRequest, GenericImage, ImageExt,
};

use rand::distributions::{Alphanumeric, DistString};
use rustls::ClientConfig as RustlsClientConfig;
use tokio::io::AsyncBufReadExt;
use tokio_postgres::{config::SslMode, Client, Config, NoTls};
use tokio_postgres_rustls::MakeRustlsConnect;

use thiserror::Error;

const POSTGRES_IMAGE: &str = "postgres";
const POSTGRES_TAG: &str = "16";
const RABBITMQ_NAME: &str = "rabbitmq";
const RABBITMQ_TAG: &str = "3.11.9-management";

pub struct TestDatabase {
    pub name: String,
    connect_config: Config,
}

impl TestDatabase {
    pub async fn drop_database(&self, client: &mut Client) {
        let query = format!("DROP DATABASE IF EXISTS \"{}\"", self.name);

        client
            .execute(&query, &[])
            .await
            .map_err(|e| format!("Error dropping database '{}': {e}", self.name))
            .unwrap();
    }

    pub async fn connect(&self) -> Result<Client, DevStackError> {
        connect_to_db(&self.connect_config).await
    }
}

pub async fn connect_to_db(config: &Config) -> Result<Client, DevStackError> {
    let client = if config.get_ssl_mode() != SslMode::Disable {
        let mut roots = rustls::RootCertStore::empty();

        for cert in rustls_native_certs::load_native_certs().expect("could not load platform certs")
        {
            roots.add(cert).unwrap();
        }

        let tls_config = RustlsClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let tls = MakeRustlsConnect::new(tls_config);

        let (client, connection) =
            config
                .connect(tls)
                .await
                .map_err(|e| DevStackError::DatabaseConnection {
                    source: e,
                    config: show_config(config),
                })?;

        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("connection error: {e}");
            }
        });

        client
    } else {
        let (client, connection) =
            config
                .connect(NoTls)
                .await
                .map_err(|e| DevStackError::DatabaseConnection {
                    source: e,
                    config: show_config(config),
                })?;

        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("connection error: {e}");
            }
        });

        client
    };

    Ok(client)
}

#[derive(Error, Debug)]
pub enum DevStackError {
    #[error("Container issue with dev stack: {0}")]
    Testcontainer(#[from] testcontainers::TestcontainersError),
    #[error("Database connection to {config:?}: {source}")]
    DatabaseConnection {
        #[source]
        source: tokio_postgres::Error,
        config: String,
    },
    #[error("Database query: {0}")]
    DatabaseQuery(tokio_postgres::Error),
}

fn show_config(config: &Config) -> String {
    let hosts = config.get_hosts();

    let host = match &hosts[0] {
        tokio_postgres::config::Host::Tcp(tcp_host) => tcp_host.clone(),
        tokio_postgres::config::Host::Unix(socket_path) => {
            socket_path.to_string_lossy().to_string()
        }
    };

    let port = config.get_ports()[0];

    let dbname = config.get_dbname().unwrap_or("");

    let sslmode = match config.get_ssl_mode() {
        SslMode::Prefer => "prefer".to_string(),
        SslMode::Disable => "disable".to_string(),
        SslMode::Require => "require".to_string(),
        _ => "<UNSUPPORTED MODE>".to_string(),
    };

    format!(
        "host={} port={} user={} dbname={} sslmode={}",
        &host,
        &port,
        config.get_user().unwrap_or(""),
        dbname,
        sslmode
    )
}

pub struct DevStack {
    pub postgres_container: ContainerAsync<GenericImage>,
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
    pub async fn start(
        postgres_config_file: &Path,
        print_output: bool,
    ) -> Result<DevStack, DevStackError> {
        let postgres_container = create_postgres_container("postgres", postgres_config_file)
            .start()
            .await
            .unwrap();

        if print_output {
            print_stdout("postgres - ".to_string(), postgres_container.stdout(true));
        }

        let rabbitmq_container = create_rabbitmq_container().start().await.unwrap();

        if print_output {
            print_stdout("rabbitmq - ".to_string(), rabbitmq_container.stdout(true));
        }

        Ok(DevStack {
            postgres_container,
            rabbitmq_container,
        })
    }

    pub async fn postgres_host(&self) -> Result<url::Host, DevStackError> {
        self.postgres_container
            .get_host()
            .await
            .map_err(DevStackError::Testcontainer)
    }

    pub async fn postgres_port(&self) -> Result<u16, DevStackError> {
        self.postgres_container
            .get_host_port_ipv4(5432)
            .await
            .map_err(DevStackError::Testcontainer)
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

    pub async fn connect_config(&self, database_name: &str) -> Result<Config, DevStackError> {
        let mut config = Config::new();

        config
            .host(self.postgres_host().await?.to_string())
            .port(self.postgres_port().await?)
            .user("postgres")
            .dbname(database_name)
            .ssl_mode(tokio_postgres::config::SslMode::Disable);

        Ok(config)
    }

    pub async fn test_database(&self) -> Result<TestDatabase, DevStackError> {
        let connect_config = self.connect_config("postgres").await?;

        let client = connect_to_db(&connect_config).await?;

        let database_name = generate_name(16);

        let query = format!("CREATE DATABASE \"{database_name}\"");

        client
            .execute(&query, &[])
            .await
            .map_err(DevStackError::DatabaseQuery)?;

        Ok(TestDatabase {
            name: database_name.clone(),
            connect_config: self.connect_config(&database_name).await?,
        })
    }
}

pub fn generate_name(len: usize) -> String {
    Alphanumeric.sample_string(&mut rand::thread_rng(), len)
}

fn create_postgres_container(name: &str, config_file: &Path) -> ContainerRequest<GenericImage> {
    GenericImage::new(POSTGRES_IMAGE, POSTGRES_TAG)
        .with_wait_for(WaitFor::message_on_stdout(
            "database system is ready to accept connections",
        ))
        .with_exposed_port(ContainerPort::Tcp(5432))
        .with_env_var("POSTGRES_HOST_AUTH_METHOD", "trust")
        .with_container_name(name)
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/postgresql/postgresql.conf",
        ))
        .with_cmd(vec!["-c", "config-file=/etc/postgresql/postgresql.conf"])
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

pub fn create_rabbitmq_container() -> ContainerRequest<RabbitMq> {
    let conf_path = concat!(env!("CARGO_MANIFEST_DIR"), "/rabbitmq.conf");
    let definitions_path = concat!(env!("CARGO_MANIFEST_DIR"), "/definitions.json");

    ContainerRequest::from(RabbitMq)
        .with_mount(Mount::bind_mount(conf_path, "/etc/rabbitmq/rabbitmq.conf"))
        .with_mount(Mount::bind_mount(
            definitions_path,
            "/etc/rabbitmq/definitions.json",
        ))
}
