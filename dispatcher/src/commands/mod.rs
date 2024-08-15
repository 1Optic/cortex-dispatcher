use thiserror::Error;

pub mod dev_stack;
pub mod service;

#[derive(Error, Debug)]
pub enum DispatcherError {
    #[error("Unexpected error: {0}")]
    Runtime(String),
}

pub type CmdResult = Result<(), DispatcherError>;

pub trait Cmd {
    fn run(&self) -> CmdResult;
}
