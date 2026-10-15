//! The host layer's error type. Deliberately transport-free: it carries no HTTP status,
//! so a host maps it into its own API dialect (claw-server folds it into `AppError`).

use std::{io, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HostError {
    #[error("storage path escapes BrowserClaw state root: {0}")]
    InvalidStoragePath(String),
    #[error("storage file not found: {0}")]
    StorageNotFound(String),
    #[error("storage json is invalid at {path}: {source}")]
    StorageCorrupt {
        path: String,
        source: serde_json::Error,
    },
    #[error("io error at {path:?}: {source}")]
    Io {
        path: Option<PathBuf>,
        source: io::Error,
    },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// A host adapter (journal, profile directory, …) failed. The original error rides
    /// along untouched so a host can recover its own type on the way back out.
    #[error(transparent)]
    Adapter(Box<dyn std::error::Error + Send + Sync>),
    #[error("{0}")]
    Internal(String),
}

impl HostError {
    /// Wraps a host adapter's own error without flattening it to text.
    #[must_use]
    pub fn adapter(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Adapter(Box::new(error))
    }
}

pub type HostResult<T> = Result<T, HostError>;

pub trait IoPath<T> {
    fn with_path(self, path: impl Into<PathBuf>) -> HostResult<T>;
}

impl<T> IoPath<T> for Result<T, io::Error> {
    fn with_path(self, path: impl Into<PathBuf>) -> HostResult<T> {
        self.map_err(|source| HostError::Io {
            path: Some(path.into()),
            source,
        })
    }
}
