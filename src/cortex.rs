use std::collections::HashMap;
use std::time::Duration;
use std::thread;

extern crate inotify;

extern crate actix;
use actix::prelude::*;
use actix::{Actor, Addr};

use inotify::{Inotify};

extern crate failure;
extern crate lapin_futures;

use crate::amqp_consumer::AmqpListener;
use crate::settings;
use crate::command_handler::CommandHandler;
use crate::sftp_downloader::{SftpDownloader, SftpDownloadDispatcher};
use crate::sftp_connection::SftpConnection;
use crate::local_source::LocalSource;

use prometheus;

pub struct Cortex {
    pub settings: settings::Settings
}

impl Cortex {
    pub fn new(settings: settings::Settings) -> Cortex {
        Cortex { settings: settings }
    }

    fn start_sftp_downloaders(sftp_sources: Vec<settings::SftpSource>) -> HashMap<String, Addr<SftpDownloader>> {
        let downloaders_map: HashMap<String, Addr<SftpDownloader>> = sftp_sources
            .iter()
            .map(|sftp_source| {
                let sftp_source_name = sftp_source.name.clone();
                let owned_sftp_source: settings::SftpSource = sftp_source.clone().clone();

                let sftp_source_settings = sftp_source.clone();

                let addr = SyncArbiter::start(sftp_source_settings.thread_count, move || {
                    let conn = loop {
                        let conn_result = SftpConnection::new(&owned_sftp_source.address.clone(), &owned_sftp_source.username.clone());

                        match conn_result {
                            Ok(c) => break c,
                            Err(e) => error!("Could not connect: {}", e)
                        }

                        thread::sleep(Duration::from_millis(1000));
                    };

                    return SftpDownloader {
                        config: sftp_source_settings.clone(),
                        sftp_connection: conn,
                        local_storage_path: String::from("/tmp")
                    };
                });

                (sftp_source_name, addr)
            })
            .collect();

        downloaders_map
    }

    fn start_metrics_collector(&mut self) -> () {
        let address = "127.0.0.1:9091";

        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(2));

                let metric_families = prometheus::gather();
                let push_result = prometheus::push_metrics(
                    "cortex-dispatcher",
                    labels! {"instance".to_owned() => "HAL-9000".to_owned(),},
                    &address,
                    metric_families,
                    Some(prometheus::BasicAuthentication {
                        username: "user".to_owned(),
                        password: "pass".to_owned(),
                    }),
                );
                
                match push_result {
                    Ok(_) => {
                        info!("Pushed metrics to Prometheus Gateway");
                    },
                    Err(e) => {
                        error!("Error pushing metrics to Prometheus Gateway: {}", e);
                    }
                }
            }
        });
    }

    pub fn run(&mut self) -> () {
        let system = actix::System::new("cortex");

        let downloaders_map = Cortex::start_sftp_downloaders(self.settings.sftp_sources.clone());

        let sftp_download_dispatcher = SftpDownloadDispatcher { downloaders_map: downloaders_map };

        let init_result = Inotify::init();

        let inotify = match init_result {
            Ok(i) => i,
            Err(e) => panic!("Could not initialize inotify: {}", e),
        };

        let local_source = LocalSource {
            sources: self.settings.directory_sources.clone(),
            inotify: inotify,
        };

        local_source.start();

        let command_handler = CommandHandler {
            sftp_download_dispatcher: sftp_download_dispatcher
        };

        let listener = AmqpListener {
            addr: self.settings.command_queue.address.clone(),
            command_handler: command_handler
        };

        self.start_metrics_collector();

        let join_handle = listener.start_consumer();

        system.run();

        join_handle.join().unwrap();
    }
}
