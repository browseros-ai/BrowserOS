//! Identity glossary — the four scopes:
//! - `ProfileId` — a stored agent profile configured by the user and persisted
//!   by `crate::services::profiles`.
//! - `SessionId` — a server-minted task handle (or a legacy transport session).
//!   Browser-target successors alias one persisted task record; `BrowserProfileId`
//!   in core identifies a native saved profile and is unrelated to `ProfileId`.
//! - `ConvoId` — one conversation, `"{client_slug}-{fun_name}"`, minted with
//!   the session and stable for its life. Legacy JSON and database records call
//!   it `agentId` and `agent_id`; canonical API responses expose profile and
//!   slug identity instead. Keys page ownership and audit attribution.
//! - `DispatchId` — one tool call.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, str::FromStr};
use ulid::Ulid;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            #[must_use]
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl FromStr for $name {
            type Err = std::convert::Infallible;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Ok(Self(value.to_string()))
            }
        }
    };
}

string_id!(SessionId);

/// A short stand-in for a session id or state handle in logs. The MCP security guidance treats
/// both as hijackable, so a log line carries 8 hex digits of their SHA-256, enough to follow one
/// id through the log, and never the id itself.
#[must_use]
pub fn log_fingerprint(id: &str) -> String {
    let digest = Sha256::digest(id.as_bytes());
    digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl SessionId {
    /// See [`log_fingerprint`].
    #[must_use]
    pub fn fingerprint(&self) -> String {
        log_fingerprint(&self.0)
    }
}
string_id!(ProfileId);
string_id!(ConvoId);

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DispatchId(String);

impl DispatchId {
    #[must_use]
    pub fn new() -> Self {
        Self(Ulid::new().to_string())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl Default for DispatchId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for DispatchId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for DispatchId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for DispatchId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_fingerprint_is_eight_hex_of_the_sha256() {
        assert_eq!(log_fingerprint("abc"), "ba7816bf");
        let id = SessionId::new("0b2f0c8e-5d0a-4c55-9b1e-6f2d6f1e7a10");
        let fingerprint = id.fingerprint();
        assert_eq!(fingerprint.len(), 8);
        assert!(!id.as_str().contains(&fingerprint));
    }
}
