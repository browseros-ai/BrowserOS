use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use claw_api::models::ApiError;
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
    Db(Box<dyn std::error::Error + Send + Sync>),
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
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Http {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn unsupported_media_type(message: impl Into<String>) -> Self {
        Self::Http {
            status: StatusCode::UNSUPPORTED_MEDIA_TYPE,
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
            Self::Io { .. } | Self::Db(_) | Self::Join(_) | Self::Internal(_) => {
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

/// Server-minted id for one HTTP request — never read from an inbound
/// header. The `request_context` middleware creates it, stores it as a
/// request extension, and echoes it back as `x-request-id`; canonical
/// error bodies embed it so a reported failure can be tied to its
/// request span in the logs.
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

/// The canonical routes' error dialect: the contract's `ApiError`
/// envelope (`code` / `message` / `requestId`), as opposed to the
/// generic `{ "error": … }` body `AppError` renders. Handlers on the
/// canonical surface must fail through this type so error responses
/// stay in-contract.
pub struct CanonicalError {
    status: StatusCode,
    body: ApiError,
}

impl CanonicalError {
    #[must_use]
    pub fn new(
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
        request_id: Option<&RequestId>,
    ) -> Self {
        let mut body = ApiError::new(code.into(), message.into());
        body.request_id = request_id.map(|request_id| request_id.0.clone());
        Self { status, body }
    }
}

impl IntoResponse for CanonicalError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

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
