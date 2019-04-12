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

pub struct Cortex {
    pub settings: settings::Settings
}

impl Cortex {
    pub fn new(settings: settings::Settings) -> Cortex {
        Cortex { settings: settings }
    }

    fn start_sftp_downloaders(sftp_downloaders: Vec<settings::SftpDownloader>, sftp_sources_hash: HashMap<String, &settings::SftpSource>) -> HashMap<String, Addr<SftpDownloader>> {
        let downloaders_map: HashMap<String, Addr<SftpDownloader>> = sftp_downloaders
            .iter()
            .map(|downloader| {
                let sftp_source_name = downloader.sftp_source.clone();
                let sftp_source = sftp_sources_hash.get(&sftp_source_name).unwrap();
                let owned_sftp_source: settings::SftpSource = sftp_source.clone().clone();

                let downloader_settings = downloader.clone();

                let addr = SyncArbiter::start(downloader_settings.thread_count, move || {
                    let conn = loop {
                        let conn_result = SftpConnection::new(&owned_sftp_source.clone());

                        match conn_result {
                            Ok(c) => break c,
                            Err(e) => error!("Could not connect: {}", e)
                        }

                        thread::sleep(Duration::from_millis(1000));
                    };

                    return SftpDownloader {
                        config: downloader_settings.clone(),
                        sftp_connection: conn,
                    };
                });

                (sftp_source_name, addr)
            })
            .collect();

        downloaders_map
    }

    pub fn run(self) -> () {
        let system = actix::System::new("cortex");

        let sftp_sources_hash: HashMap<String, &settings::SftpSource> = self.settings
            .sftp_sources
            .iter()
            .map(|sftp_source| (sftp_source.name.clone(), sftp_source))
            .collect();

        let downloaders_map = Cortex::start_sftp_downloaders(self.settings.sftp_downloaders, sftp_sources_hash);

        let sftp_download_dispatcher = SftpDownloadDispatcher { downloaders_map: downloaders_map };

        let init_result = Inotify::init();

        let inotify = match init_result {
            Ok(i) => i,
            Err(e) => panic!("Could not initialize inotify: {}", e),
        };

        let local_source = LocalSource {
            sources: self.settings.directory_sources,
            inotify: inotify,
        };

        local_source.start();

        let command_handler = CommandHandler {
            sftp_download_dispatcher: sftp_download_dispatcher
        };

        let listener = AmqpListener {
            addr: self.settings.command_queue.address,
            command_handler: command_handler
        };

        let join_handle = listener.start_consumer();

        system.run();

        join_handle.join().unwrap();
    }
}
