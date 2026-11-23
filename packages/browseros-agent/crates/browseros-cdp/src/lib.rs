#[cfg(test)]
mod alloc_count;
mod client;
mod connection_loss;
mod discovery;
mod error;
mod events;
mod generated;
mod inbound;
mod raw_reply;
mod targeted;

pub use client::{CdpClient, ConnectOptions, ReconnectPolicy};
pub use connection_loss::ConnectionLoss;
pub use discovery::discover_websocket_url;
pub use error::CdpError;
pub use events::{CdpEvent, EventStream};
pub use generated::*;
pub use raw_reply::RawReply;
pub use targeted::TargetedEvents;

use serde::{Deserialize, Serialize};
use std::{fmt, hash::Hash};

#[derive(Debug, Clone, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for SessionId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SessionId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
