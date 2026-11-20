use std::fmt;

/// Unified error type for the core engine.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("disabled: {0}")]
    Disabled(String),
    #[error("wav: {0}")]
    Wav(String),
    #[error("qa blocked export: {0}")]
    QaBlocked(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("database: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn invalid(msg: impl fmt::Display) -> Error {
    Error::Invalid(msg.to_string())
}

pub(crate) fn not_found(what: &str, id: &str) -> Error {
    Error::NotFound(format!("{what} '{id}'"))
}
