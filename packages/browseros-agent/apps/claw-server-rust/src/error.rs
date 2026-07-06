use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use std::{io, path::PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{message}")]
    Http { status: StatusCode, message: String },
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
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),
    #[error("{0}")]
    Internal(String),
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
}

impl AppError {
    #[must_use]
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::Http {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::Http {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn gone(message: impl Into<String>) -> Self {
        Self::Http {
            status: StatusCode::GONE,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Http {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Http { status, .. } => *status,
            Self::InvalidStoragePath(_) | Self::StorageCorrupt { .. } | Self::Json(_) => {
                StatusCode::BAD_REQUEST
            }
            Self::StorageNotFound(_) => StatusCode::NOT_FOUND,
            Self::Io { .. } | Self::Sql(_) | Self::Join(_) | Self::Internal(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let message = self.to_string();
        (status, Json(ErrorBody { error: &message })).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;

pub trait IoPath<T> {
    fn with_path(self, path: impl Into<PathBuf>) -> AppResult<T>;
}

impl<T> IoPath<T> for Result<T, io::Error> {
    fn with_path(self, path: impl Into<PathBuf>) -> AppResult<T> {
        self.map_err(|source| AppError::Io {
            path: Some(path.into()),
            source,
        })
    }
}
