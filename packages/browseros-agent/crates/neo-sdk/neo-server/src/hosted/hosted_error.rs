use std::fmt::Display;

/// A failure to open or close the host; SDK calls themselves always answer through the bridge.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{0}")]
pub struct HostedError(pub(crate) String);

impl HostedError {
    pub(super) fn from_error(error: impl Display) -> Self {
        Self(error.to_string())
    }
}
