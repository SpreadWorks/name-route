use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Docker error: {0}")]
    Docker(#[from] bollard::errors::Error),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Routing error: no backend for {protocol}:{key}")]
    NoBackend { protocol: String, key: String },

    #[error("Connection error: {0}")]
    Connection(String),
}

pub type Result<T> = std::result::Result<T, Error>;
