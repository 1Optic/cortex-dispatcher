extern crate clap;
extern crate config;

#[macro_use]
extern crate log;
extern crate env_logger;

mod base_types;
mod cmd;
mod cortex;
mod directory_source;
mod directory_target;
mod event;
mod metrics;
mod metrics_collector;
mod settings;
mod sftp_source;
mod http_server;

use settings::Settings;

#[macro_use]
extern crate serde_derive;

extern crate postgres;

extern crate chrono;
extern crate tee;
extern crate sha2;

#[macro_use]
extern crate prometheus;

#[macro_use]
extern crate lazy_static;

extern crate cortex_core;

fn main() {
    let matches = cmd::app().get_matches();

    let mut env_logger_builder = env_logger::builder();

    if matches.is_present("service") {
        env_logger_builder.default_format_timestamp(false);
    }

    env_logger_builder.init();

    let config_file = matches
        .value_of("config")
        .unwrap_or("/etc/cortex/cortex.yaml");

    let mut settings = config::Config::new();

    info!("Loading configuration");

    let merge_result = settings
        .merge(config::File::new(config_file, config::FileFormat::Yaml));

    match merge_result {
        Ok(_config) => {
            info!("Configuration loaded from file {}", config_file);
        },
        Err(e) => {
            error!("Error loading configuration: {}", e);
            ::std::process::exit(1);
        }
    }

    let into_result = settings.try_into();

    let settings: Settings = match into_result {
        Ok(s) => s,
        Err(e) => {
            error!("Error loading configuration: {}", e);
            ::std::process::exit(1);
        }
    };

    info!("Configuration loaded");

    cortex::run(settings);
}
