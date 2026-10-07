//! rtcl-data 统一错误类型。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DataError {
    #[error("invalid url: {0}")]
    Url(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("yaml error: {0}")]
    Yaml(String),
    #[error("invalid argument: {0}")]
    InvalidArg(String),
}

pub type Result<T> = std::result::Result<T, DataError>;
