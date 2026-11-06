//! A route's failure, and the one mapping from each crate's errors to the contract's codes.
//!
//! A handler returns an [`ApiError`] and never sees the request id: `IntoResponse` leaves the
//! error on the response, and `response_id` writes the contract's `Error` body with the id.
//! A failure inside Neo keeps its cause, which `response_id` logs under that id; the host gets
//! only a generic message for it, so storage details never reach the wire.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use browseros_capture::CaptureError;
use browseros_takeover::PartError;
use browseros_trace::{PrivacyError, TraceError};
use std::borrow::Cow;

/// The contract's error codes (`Error.code`) this service sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ErrorCode {
    InvalidRequest,
    Unauthorized,
    NotFound,
    TabNotFound,
    PagePrivate,
    Withheld,
    Expired,
    TakeoverLock,
    PrivacyUnavailable,
    BrowserUnavailable,
    InternalError,
    FitRestored,
    DisplayPaused,
    DisplayUnavailable,
    TakeoverReleased,
}

impl ErrorCode {
    /// The wire name.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Unauthorized => "unauthorized",
            Self::NotFound => "not_found",
            Self::TabNotFound => "tab_not_found",
            Self::PagePrivate => "page_private",
            Self::Withheld => "withheld",
            Self::Expired => "expired",
            Self::TakeoverLock => "takeover_lock",
            Self::PrivacyUnavailable => "privacy_unavailable",
            Self::BrowserUnavailable => "browser_unavailable",
            Self::InternalError => "internal_error",
            Self::FitRestored => "fit_restored",
            Self::DisplayPaused => "display_paused",
            Self::DisplayUnavailable => "display_unavailable",
            Self::TakeoverReleased => "takeover_released",
        }
    }
}

/// A failed request: its status, the contract's code, a message for the host and, for a
/// failure inside Neo, the cause to log.
#[derive(Clone, Debug)]
pub(crate) struct ApiError {
    pub(crate) status: StatusCode,
    pub(crate) code: ErrorCode,
    pub(crate) message: Cow<'static, str>,
    /// Logged with the request id, never sent.
    pub(crate) cause: Option<String>,
}

impl ApiError {
    fn new(status: StatusCode, code: ErrorCode, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            cause: None,
        }
    }

    /// 400 `invalid_request`.
    pub(crate) fn invalid(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidRequest, message)
    }

    /// 404 `not_found`.
    pub(crate) fn not_found(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, message)
    }

    /// 404 `tab_not_found`: no open tab has the id.
    pub(crate) fn tab_not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            ErrorCode::TabNotFound,
            "no open tab has this id",
        )
    }

    /// 409 `page_private`: the tab is private, so nothing of it is captured.
    pub(crate) fn page_private() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::PagePrivate,
            "the tab is private; nothing of it is captured",
        )
    }

    /// 410 `withheld`: the image was not kept because its tab was private.
    pub(crate) fn withheld(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::GONE, ErrorCode::Withheld, message)
    }

    /// 410 `expired`: retention deleted the image.
    pub(crate) fn expired(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::GONE, ErrorCode::Expired, message)
    }

    /// 401 `unauthorized`.
    pub(crate) fn unauthorized(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized, message)
    }

    /// 500 `internal_error`: a failure inside Neo. `cause` is logged, and the host gets only a
    /// generic message.
    pub(crate) fn internal(cause: impl Into<String>) -> Self {
        Self {
            cause: Some(cause.into()),
            ..Self::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::InternalError,
                "internal error",
            )
        }
    }

    /// The contract's error for a status no handler described, such as axum's own 405 or a
    /// body over its size limit. `None` for a status that is not an error.
    pub(crate) fn for_status(status: StatusCode) -> Option<Self> {
        if !status.is_client_error() && !status.is_server_error() {
            return None;
        }
        let code = match status {
            StatusCode::UNAUTHORIZED => ErrorCode::Unauthorized,
            StatusCode::NOT_FOUND => ErrorCode::NotFound,
            status if status.is_server_error() => ErrorCode::InternalError,
            _ => ErrorCode::InvalidRequest,
        };
        let message = status.canonical_reason().unwrap_or("the request failed");
        Some(Self::new(status, code, message))
    }
}

/// Every trace failure is Neo's own: the file, the writer, a corrupt row. A malformed cursor,
/// which the store also reports as `Corrupt`, is refused by `query` before the store sees it.
impl From<TraceError> for ApiError {
    fn from(error: TraceError) -> Self {
        Self::internal(error.to_string())
    }
}

/// The flag still holds in memory, so captures stay refused, but it is not on disk: the host
/// must not act as if the tab were durably private.
impl From<PrivacyError> for ApiError {
    fn from(error: PrivacyError) -> Self {
        Self {
            cause: Some(error.to_string()),
            ..Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::PrivacyUnavailable,
                "the private flag could not be committed",
            )
        }
    }
}

/// A takeover part's failure. The parts know nothing of HTTP, so this is where each becomes
/// the contract's status and code.
impl From<PartError> for ApiError {
    fn from(error: PartError) -> Self {
        match error {
            PartError::NotFound => Self::not_found("nothing has this id"),
            PartError::TabNotFound(_) => Self::tab_not_found(),
            PartError::TakeoverLock => Self::new(
                StatusCode::CONFLICT,
                ErrorCode::TakeoverLock,
                "this lock belongs to a takeover; release the takeover instead",
            ),
            PartError::Browser(cause) => Self {
                cause: Some(cause.to_string()),
                ..Self::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    ErrorCode::BrowserUnavailable,
                    "Neo could not reach the browser",
                )
            },
            PartError::TimedOut(step) => Self {
                cause: Some(format!("{step} timed out")),
                ..Self::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    ErrorCode::BrowserUnavailable,
                    "the browser did not answer in time",
                )
            },
            // Xvnc is the display: the screen every display shows and every fit resizes.
            PartError::Screen(cause) => Self {
                cause: Some(cause.to_string()),
                ..Self::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    ErrorCode::DisplayUnavailable,
                    "Neo's screen did not answer as it should",
                )
            },
            PartError::FitRestored => Self::new(
                StatusCode::CONFLICT,
                ErrorCode::FitRestored,
                "the fit was restored",
            ),
            PartError::UnusableViewer(_) => {
                Self::invalid("`width` and `height` must each be at least 100")
            }
            PartError::DisplayPaused => Self::new(
                StatusCode::CONFLICT,
                ErrorCode::DisplayPaused,
                "view displays are paused while the owner has control or a private tab is in \
                 front; try again later",
            ),
            PartError::TakeoverReleased => Self::new(
                StatusCode::CONFLICT,
                ErrorCode::TakeoverReleased,
                "the takeover was released",
            ),
            PartError::Privacy(error) => error.into(),
        }
    }
}

/// A live view the capture hub could not open. The hub knows a tab as a "session" whose only
/// tab is that tab (`live_tabs`), so an unknown session is a tab that is not open, and a
/// session that is not live is a private tab. Every case is answered before a byte is sent.
impl From<CaptureError> for ApiError {
    fn from(error: CaptureError) -> Self {
        match error {
            CaptureError::SessionNotFound | CaptureError::NoOpenTab | CaptureError::TabNotFound => {
                Self::tab_not_found()
            }
            CaptureError::SessionNotLive => Self::page_private(),
            CaptureError::Browser(cause) => Self {
                cause: Some(cause.to_string()),
                ..Self::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    ErrorCode::BrowserUnavailable,
                    "Neo could not start capturing the tab",
                )
            },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = self.status.into_response();
        response.extensions_mut().insert(self);
        response
    }
}
