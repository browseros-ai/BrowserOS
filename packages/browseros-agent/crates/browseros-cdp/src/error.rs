#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum CdpError {
    #[error("CDP error: {message}")]
    Protocol { code: i64, message: String },
    #[error("CDP request timed out: {method}")]
    Timeout { method: String },
    #[error("CDP connection lost")]
    ConnectionLost,
    #[error("CDP not connected")]
    NotConnected,
    #[error("no session with given id")]
    SessionGone,
    #[error("CDP discovery failed: {0}")]
    Discovery(String),
    #[error("CDP transport error: {0}")]
    Transport(String),
    #[error("CDP JSON error: {0}")]
    Json(String),
}

impl CdpError {
    #[must_use]
    pub fn from_protocol(code: i64, message: String) -> Self {
        if message.contains("No session with given id")
            || message.contains("Session with given id not found")
        {
            Self::SessionGone
        } else {
            Self::Protocol { code, message }
        }
    }
}

impl From<serde_json::Error> for CdpError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value.to_string())
    }
}
