use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum DispatcherError {
    #[error("Connection error: {0}")]
    ConnectionError(String),
    #[error("Disconnected: {0}")]
    DisconnectedError(String),
    #[error("No such file")]
    NoSuchFile,
    #[error("Connection interrupted: {0}")]
    ConnectionInterrupted(String),
    #[error("Persistence error: {0}")]
    PersistenceError(String),
    #[error("File error: {0}")]
    FileError(String),
    #[error("Database error: {0}")]
    DatabaseError(String),
    #[error("Other dispatcher error: {0}")]
    OtherError(String),
}
