use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("vfs: {0}")]
    Vfs(String),

    #[error("config: {0}")]
    Config(String),

    #[error("invalid path: {0}")]
    InvalidPath(String),

    #[error("invalid key: {0}")]
    InvalidKey(String),

    #[error("operation cancelled")]
    Cancelled,

    #[error("not supported")]
    NotSupported,

    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync + 'static>),
}

pub type Result<T> = std::result::Result<T, Error>;
