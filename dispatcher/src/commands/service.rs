use std::io::Write;

use clap::Parser;
use log::{error, info};

use crate::commands::{Cmd, CmdResult};
use crate::dispatcher;
use crate::DispatcherError;

#[derive(Parser, Debug)]
pub struct ServiceOpt {
    /// Path to config file
    #[arg(short, long)]
    config: Option<String>,

    /// Show example config
    #[arg(short, long)]
    example_config: bool,
}

impl Cmd for ServiceOpt {
    fn run(&self) -> CmdResult {
        let mut env_logger_builder = env_logger::builder();

        env_logger_builder
            .format(|buf, record| writeln!(buf, "{}  {}", record.level(), record.args()));

        env_logger_builder.init();

        if self.example_config {
            println!(
                "{}",
                serde_yaml::to_string(&crate::settings::Settings::default()).unwrap()
            );
            ::std::process::exit(0);
        }

        let config_file = self
            .config
            .clone()
            .unwrap_or("/etc/cortex/cortex.yaml".into());

        info!("Loading configuration");

        let merge_result = config::Config::builder()
            .add_source(config::File::new(&config_file, config::FileFormat::Yaml))
            .build();

        let settings = match merge_result {
            Ok(config) => {
                info!("Configuration loaded from file {}", config_file);

                config.try_deserialize().unwrap()
            }
            Err(e) => {
                error!("Error merging configuration: {}", e);
                ::std::process::exit(1);
            }
        };

        info!("Configuration loaded");

        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(dispatcher::run(settings));

        match result {
            Ok(_) => Ok(()),
            Err(e) => Err(DispatcherError::Runtime(format!("{}", e))),
        }
    }
}
