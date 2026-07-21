//! Identity glossary — the four scopes:
//! - `ProfileId` — a stored agent profile configured by the user and persisted
//!   by `crate::services::profiles`.
//! - `SessionId` — one MCP transport connection, minted per handshake.
//! - `ConvoId` — one conversation, `"{client_slug}-{fun_name}"`, minted with
//!   the session and stable for its life. Legacy JSON and database records call
//!   it `agentId` and `agent_id`; canonical API responses expose profile and
//!   slug identity instead. Keys page ownership and audit attribution.
//! - `DispatchId` — one tool call.

use serde::{Deserialize, Serialize};
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
