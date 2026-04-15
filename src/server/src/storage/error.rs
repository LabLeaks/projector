/**
@module PROJECTOR.SERVER.STORE_ERROR
Owns the shared storage error model and backend-error conversions exposed by the server storage boundary.
*/
// @fileimplements PROJECTOR.SERVER.STORE_ERROR
use std::fmt;

#[derive(Debug, Clone)]
pub struct StoreError {
    code: String,
    is_conflict: bool,
    message: String,
}

impl StoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_request".to_owned(),
            is_conflict: false,
            message: message.into(),
        }
    }

    pub fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            is_conflict: true,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn is_conflict(&self) -> bool {
        self.is_conflict
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(value: std::io::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<tokio_postgres::Error> for StoreError {
    fn from(value: tokio_postgres::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::new(value.to_string())
    }
}
