use std::process::ExitCode;

use commands::{dev_stack::DevStackOpt, service::ServiceOpt, DispatcherError};

mod base_types;
mod commands;
mod directory_source;
mod directory_target;
mod dispatcher;
mod event;
mod local_storage;
mod metrics;
mod persistence;
mod settings;
mod sftp_command_consumer;
mod sftp_downloader;

use clap::{Parser, Subcommand};

use crate::commands::Cmd;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None, arg_required_else_help = true)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Start Cortex Dispatcher service")]
    Service(ServiceOpt),
    #[command(about = "Start development containers")]
    DevStack(DevStackOpt),
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Some(Command::Service(service)) => service.run(),
        Some(Command::DevStack(dev_stack)) => dev_stack.run(),
        None => return ExitCode::FAILURE,
    };

    if let Err(e) = result {
        println!("{}", e);
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
