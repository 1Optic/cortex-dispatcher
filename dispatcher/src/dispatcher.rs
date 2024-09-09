use futures::future::join_all;
use rustls::client::danger::HandshakeSignatureValid;
use std::collections::HashMap;
use std::iter::Iterator;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::DigitallySignedStruct;

use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::sync::watch;

use futures::stream::StreamExt;

use r2d2_postgres::PostgresConnectionManager;

use signal_hook_tokio::Signals;

use crossbeam_channel::{bounded, Receiver, Sender};

use log::{debug, error, info};

use cortex_core::{wait_for, SftpDownload};

use crate::base_types::{Connection, RabbitMQNotifier, Source, Target};

#[cfg(target_os = "linux")]
use crate::directory_source::start_directory_sources;
use crate::directory_source::{start_directory_sweep, start_local_intake_thread};

use crate::directory_target::handle_file_event;
use crate::event::{EventDispatcher, FileEvent};
use crate::local_storage::LocalStorage;
use crate::persistence::{self};
use crate::persistence::{PostgresAsyncPersistence, PostgresPersistence};
use crate::settings;
use crate::sftp_command_consumer;
use crate::sftp_downloader;

pub async fn target_directory_handler<T>(
    tokio_persistence: PostgresAsyncPersistence<T>,
    settings: settings::Settings,
    stop_receiver: watch::Receiver<()>,
    targets: Arc<Mutex<HashMap<String, Arc<Target>>>>,
) where
    T: postgres::tls::MakeTlsConnect<tokio_postgres::Socket> + Clone + 'static + Sync + Send,
    T::TlsConnect: Send,
    T::Stream: Send + Sync,
    <T::TlsConnect as postgres::tls::TlsConnect<tokio_postgres::Socket>>::Future: Send,
{
    settings.directory_targets.iter().for_each(|target_conf| {
        let persistence = tokio_persistence.clone();
        let (sender, mut receiver) = unbounded_channel::<FileEvent>();

        let c_target_conf = target_conf.clone();
        let d_target_conf = target_conf.clone();

        match c_target_conf.notify {
            Some(conf) => match conf {
                settings::Notify::RabbitMQ(notify_conf) => {
                    let fut = async move {
                        debug!("Connecting notifier to directory target stream");

                        let mut notify = RabbitMQNotifier::from(&notify_conf);

                        let routing_key = notify_conf.routing_key.clone();

                        while let Some(file_event) = receiver.recv().await {
                            match handle_file_event(&d_target_conf, file_event, persistence.clone())
                                .await
                            {
                                Ok(result_event) => {
                                    debug!("Notifying with AMQP routing key {}", &routing_key);

                                    match notify.notify(result_event).await {
                                        Err(e) => error!("{e}"),
                                        Ok(_) => debug!("published"),
                                    };
                                }
                                Err(e) => {
                                    error!("Error handling event for directory target: {}", &e);
                                }
                            }
                        }
                    };

                    let mut stop_receiver_clone = stop_receiver.clone();

                    tokio::spawn(async move {
                        tokio::select!(
                            _a = fut => (),
                            _b = stop_receiver_clone.changed() => ()
                        )
                    })
                }
            },
            None => {
                let fut = async move {
                    while let Some(file_event) = receiver.recv().await {
                        if let Err(e) =
                            handle_file_event(&d_target_conf, file_event, persistence.clone()).await
                        {
                            error!("Error handling event for directory target: {}", &e);
                        }
                    }
                };

                let mut stop_receiver_clone = stop_receiver.clone();

                tokio::spawn(async move {
                    tokio::select!(
                        _a = fut => (),
                        _b = stop_receiver_clone.changed()=> ()
                    )
                })
            }
        };

        let target = Arc::new(Target {
            name: c_target_conf.name.clone(),
            sender,
        });

        match targets.lock() {
            Ok(mut guard) => {
                guard.insert(target_conf.name.clone(), target);
            }
            Err(e) => error!(
                "Could not get lock on targets hash for adding Target: {}",
                e
            ),
        }
    });
}

type SftpJoinHandle = thread::JoinHandle<std::result::Result<(), sftp_downloader::Error>>;

struct SftpSourceSend {
    pub sftp_source: settings::SftpSource,
    pub cmd_sender: Sender<(u64, SftpDownload)>,
    pub cmd_receiver: Receiver<(u64, SftpDownload)>,
    pub file_event_sender: tokio::sync::mpsc::UnboundedSender<FileEvent>,
    pub stop_receiver: tokio::sync::watch::Receiver<()>,
}

async fn sftp_sources_handler<T>(
    settings: settings::Settings,
    sftp_join_handles: Arc<Mutex<Vec<SftpJoinHandle>>>,
    sftp_source_senders: Vec<SftpSourceSend>,
    stop_flag: Arc<AtomicBool>,
    local_storage: LocalStorage<T>,
    persistence: T,
) -> Result<(), sftp_command_consumer::ConsumeError>
where
    T: persistence::Persistence + Clone + Sync + Send + 'static,
{
    debug!(
        "Connecting to AMQP service at {}",
        &settings.command_queue.address
    );

    debug!("Connected to AMQP service");

    let mut stream_join_handles: Vec<
        tokio::task::JoinHandle<Result<(), sftp_command_consumer::ConsumeError>>,
    > = Vec::new();

    for mut channels in sftp_source_senders {
        let (ack_sender, ack_receiver) = async_channel::bounded(100);

        // For now only log the ack messages
        tokio::spawn(ack_receiver.for_each(|ack_message| async move {
            debug!("Ack received from SftpDownloader: {:?}", &ack_message);
        }));

        for n in 0..channels.sftp_source.thread_count {
            debug!(
                "Starting SFTP download thread '{}'",
                &channels.sftp_source.name
            );

            let join_handle = sftp_downloader::SftpDownloader::start(
                stop_flag.clone(),
                channels.cmd_receiver.clone(),
                ack_sender.clone(),
                channels.sftp_source.clone(),
                channels.file_event_sender.clone(),
                local_storage.clone(),
                persistence.clone(),
            );

            let guard = sftp_join_handles.lock();

            guard.unwrap().push(join_handle);

            info!(
                "Started SFTP download thread for source '{}' ({}/{})",
                &channels.sftp_source.name,
                n + 1,
                channels.sftp_source.thread_count
            );
        }

        debug!("Spawning AMQP stream task '{}'", &channels.sftp_source.name);

        let consume_future = sftp_command_consumer::start(
            settings.command_queue.address.clone(),
            channels.sftp_source.name.clone(),
            channels.cmd_sender.clone(),
        );

        let source_name = channels.sftp_source.name.clone();

        stream_join_handles.push(tokio::spawn(async move {
            tokio::select!(
                a = consume_future => a,
                _b = channels.stop_receiver.changed() => {
                    debug!("Interrupted SFTP command consumer stream '{}'", &source_name);
                    Ok(())
                }
            )
        }));
    }

    // Await on futures so that the AMQP connection does not get destroyed.
    let _stream_results = join_all(stream_join_handles).await;

    Ok::<(), sftp_command_consumer::ConsumeError>(())
}

/// Start the streams that dispatch messages from sources to targets
///
/// All connections from the same source are bundled into one stream that
/// dispatches to all targets of those connections, because there is only one
/// receiver per source.
pub fn start_dispatch_streams(
    sources: Vec<Source>,
    connections: Vec<Connection>,
) -> Vec<Option<tokio::task::JoinHandle<Result<(), ()>>>> {
    sources
        .into_iter()
        .map(
            |source| -> Option<tokio::task::JoinHandle<Result<(), ()>>> {
                // Filter connections to this source
                let source_connections: Vec<Connection> = connections
                    .iter()
                    .filter(|c| c.source_name == source.name)
                    .cloned()
                    .collect();

                debug!(
                    "Spawing local event dispatcher task for source '{}'",
                    &source.name
                );

                Some(tokio::spawn(dispatch_stream(source, source_connections)))
            },
        )
        .collect()
}

pub async fn run(settings: settings::Settings) -> Result<(), anyhow::Error> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|e| anyhow::anyhow!("Could not initialize default TLS provider: {e:?}"))?;

    // List of targets with their file event channels
    let targets: Arc<Mutex<HashMap<String, Arc<Target>>>> = Arc::new(Mutex::new(HashMap::new()));

    // List of sources with their file event channels
    let mut sources: Vec<Source> = Vec::new();

    let postgres_config: postgres::Config = settings.postgresql.url.parse()?;

    #[derive(Debug)]
    pub struct NoCertificateVerification(CryptoProvider);

    impl NoCertificateVerification {
        pub fn new(provider: CryptoProvider) -> Self {
            Self(provider)
        }
    }

    impl rustls::client::danger::ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp: &[u8],
            _now: UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls12_signature(
                message,
                cert,
                dss,
                &self.0.signature_verification_algorithms,
            )
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls13_signature(
                message,
                cert,
                dss,
                &self.0.signature_verification_algorithms,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }

    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();

    config
        .dangerous()
        .set_certificate_verifier(Arc::new(NoCertificateVerification::new(
            rustls::crypto::ring::default_provider(),
        )));

    let tls = tokio_postgres_rustls::MakeRustlsConnect::new(config);
    let connection_manager = PostgresConnectionManager::new(postgres_config, tls.clone());

    let persistence = PostgresPersistence::new(connection_manager).map_err(anyhow::Error::msg)?;

    let postgres_config: tokio_postgres::Config = settings.postgresql.url.parse()?;

    let tokio_connection_manager =
        bb8_postgres::PostgresConnectionManager::new(postgres_config, tls);

    let tokio_persistence = PostgresAsyncPersistence::new(tokio_connection_manager).await;

    let (stop_sender, stop_receiver) = watch::channel(());

    tokio::spawn(target_directory_handler(
        tokio_persistence,
        settings.clone(),
        stop_receiver.clone(),
        targets.clone(),
    ));

    let local_storage = LocalStorage::new(&settings.storage.directory, persistence.clone());

    let (local_intake_sender, local_intake_receiver) = std::sync::mpsc::channel();

    let mut senders: HashMap<String, UnboundedSender<FileEvent>> = HashMap::new();

    settings
        .directory_sources
        .iter()
        .for_each(|directory_source| {
            let (sender, receiver) = unbounded_channel();

            sources.push(Source {
                name: directory_source.name.clone(),
                receiver,
            });

            senders.insert(directory_source.name.clone(), sender);
        });

    let event_dispatcher = EventDispatcher { senders };

    // Create a lookup table for directory sources that can be used by the intake
    // thread
    let directory_source_map: HashMap<String, settings::DirectorySource> = (settings
        .directory_sources)
        .iter()
        .map(|d| (d.name.clone(), d.clone()))
        .collect();

    let stop_flag = Arc::new(AtomicBool::new(false));

    let local_intake_handle = start_local_intake_thread(
        local_intake_receiver,
        event_dispatcher,
        local_storage.clone(),
        directory_source_map,
        stop_flag.clone(),
    );

    #[cfg(target_os = "linux")]
    let directory_sources_join_handle = start_directory_sources(
        settings.directory_sources.clone(),
        local_intake_sender.clone(),
        stop_flag.clone(),
    );

    #[cfg(target_os = "linux")]
    info!("Configure stopping of inotify handlers");

    let directory_sweep_join_handle = start_directory_sweep(
        settings.directory_sources.clone(),
        local_intake_sender,
        settings.scan_interval,
        stop_flag.clone(),
    );

    let sftp_join_handles: Arc<Mutex<Vec<SftpJoinHandle>>> = Arc::new(Mutex::new(Vec::new()));

    let (sftp_source_senders, mut sftp_sources): (Vec<SftpSourceSend>, Vec<Source>) = settings
        .sftp_sources
        .iter()
        .map(|sftp_source| {
            let (cmd_sender, cmd_receiver) = bounded::<(u64, SftpDownload)>(10);
            let (file_event_sender, file_event_receiver) = unbounded_channel();

            let sftp_source_send = SftpSourceSend {
                sftp_source: sftp_source.clone(),
                cmd_sender,
                cmd_receiver,
                file_event_sender,
                stop_receiver: stop_receiver.clone(),
            };

            let source = Source {
                name: sftp_source.name.clone(),
                receiver: file_event_receiver,
            };

            (sftp_source_send, source)
        })
        .unzip();

    sources.append(&mut sftp_sources);

    let _sftp_sources_join_handle = tokio::spawn(sftp_sources_handler(
        settings.clone(),
        sftp_join_handles.clone(),
        sftp_source_senders,
        stop_flag.clone(),
        local_storage,
        persistence,
    ));

    let connections = settings
        .connections
        .iter()
        .filter_map(|conn_conf| -> Option<Connection> {
            let target = match targets.lock() {
                Ok(guard) => match guard.get(&conn_conf.target) {
                    Some(target) => target.clone(),
                    None => {
                        error!("No target found matching name '{}'", &conn_conf.target);
                        return None;
                    }
                },
                Err(e) => {
                    error!("Could not lock the targets Arc for getting a target: {}", e);
                    return None;
                }
            };

            Some(Connection {
                source_name: conn_conf.source.clone(),
                target,
                filter: conn_conf.filter.clone(),
            })
        })
        .collect();

    // Start the streams that dispatch messages from sources to targets
    let _stream_join_handles = start_dispatch_streams(sources, connections);

    let signals = Signals::new([
        signal_hook::consts::signal::SIGHUP,
        signal_hook::consts::signal::SIGTERM,
        signal_hook::consts::signal::SIGINT,
        signal_hook::consts::signal::SIGQUIT,
    ])?;

    let signal_handler_join_handle = tokio::spawn(async move {
        let mut signals = signals.fuse();

        while let Some(signal) = signals.next().await {
            match signal {
                signal_hook::consts::signal::SIGHUP => {
                    // Reload configuration
                    // Reopen the log file
                }
                signal_hook::consts::signal::SIGTERM
                | signal_hook::consts::signal::SIGINT
                | signal_hook::consts::signal::SIGQUIT => {
                    info!("Stopping dispatcher");
                    stop_flag.swap(true, Ordering::Relaxed);
                    if let Err(e) = stop_sender.send(()) {
                        error!("Could not send stop signal: {e}");
                    }
                    break;
                }
                _ => unreachable!(),
            }
        }
    });

    // Wait until all tasks have finished
    let _result = signal_handler_join_handle.await;

    info!("Tokio runtime shutdown");

    #[cfg(target_os = "linux")]
    wait_for(directory_sources_join_handle, "directory sources");

    wait_for(local_intake_handle, "local intake");

    wait_for(directory_sweep_join_handle, "directory sweep");

    Arc::try_unwrap(sftp_join_handles)
        .expect("still users of handles")
        .into_inner()
        .unwrap()
        .into_iter()
        .for_each(|jh| {
            wait_for(jh, "sftp download");
        });

    Ok(())
}

async fn dispatch_stream(mut source: Source, connections: Vec<Connection>) -> Result<(), ()> {
    while let Some(file_event) = source.receiver.recv().await {
        debug!(
            "FileEvent for {} connections, from {}: {}",
            connections.len(),
            &source.name,
            file_event.path.to_string_lossy()
        );

        connections
            .deref()
            .iter()
            .filter(|c| match &c.filter {
                Some(f) => f.file_matches(&file_event.path),
                None => true,
            })
            .for_each(|c| {
                info!("Sending FileEvent to target {}", &c.target.name);

                let send_result = c.target.sender.send(file_event.clone());

                match send_result {
                    Ok(_) => (),
                    Err(e) => {
                        // Could not send file event to target
                        // TODO: Implement retry mechanism
                        error!("Could not send event to target handler: {}", e);
                    }
                }
            });
    }

    debug!("End of dispatch stream '{}'", &source.name);

    Ok(())
}
