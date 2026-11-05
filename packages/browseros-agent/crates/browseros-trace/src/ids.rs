//! The ids every record carries. A run id is a UUIDv7, so ids sort by start time and keep index
//! locality. An operation id is its run plus a per-run counter, written `{run}.{seq}`, so an
//! operation id names its run and sorts in start order without a lookup.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// One `run(code)` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RunId(Uuid);

/// One agent session (the task handle `run` returns), which owns pages, runs and privacy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(Uuid);

/// One SDK operation: the run it belongs to and its admission order within that run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OperationId {
    /// The run this operation belongs to.
    pub run: RunId,
    /// Its place in the run's admission order, from 1.
    pub seq: u32,
}

/// A string that is not a valid id.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} id {value:?}")]
pub struct IdError {
    kind: &'static str,
    value: String,
}

impl RunId {
    /// A new, time-ordered run id.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// The id as a UUID.
    #[must_use]
    pub fn uuid(self) -> Uuid {
        self.0
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionId {
    /// A new, time-ordered session id.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// A session id the dispatcher already holds.
    #[must_use]
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// The id as a UUID.
    #[must_use]
    pub fn uuid(self) -> Uuid {
        self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl OperationId {
    /// The `seq`th operation of `run`.
    #[must_use]
    pub fn new(run: RunId, seq: u32) -> Self {
        Self { run, seq }
    }
}

impl From<Uuid> for RunId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(f)
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(f)
    }
}

impl fmt::Display for OperationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.run, self.seq)
    }
}

impl FromStr for RunId {
    type Err = IdError;
    fn from_str(value: &str) -> Result<Self, IdError> {
        Uuid::parse_str(value).map(Self).map_err(|_| IdError {
            kind: "run",
            value: value.to_string(),
        })
    }
}

impl FromStr for SessionId {
    type Err = IdError;
    fn from_str(value: &str) -> Result<Self, IdError> {
        Uuid::parse_str(value).map(Self).map_err(|_| IdError {
            kind: "session",
            value: value.to_string(),
        })
    }
}

impl FromStr for OperationId {
    type Err = IdError;
    fn from_str(value: &str) -> Result<Self, IdError> {
        let invalid = || IdError {
            kind: "operation",
            value: value.to_string(),
        };
        let (run, seq) = value.rsplit_once('.').ok_or_else(invalid)?;
        Ok(Self {
            run: run.parse().map_err(|_| invalid())?,
            seq: seq.parse().map_err(|_| invalid())?,
        })
    }
}

macro_rules! string_serde {
    ($ty:ty) => {
        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

string_serde!(RunId);
string_serde!(SessionId);
string_serde!(OperationId);

#[cfg(test)]
mod tests {
    use super::{OperationId, RunId, SessionId};

    #[test]
    fn an_operation_id_round_trips_through_its_text_form() {
        let id = OperationId::new(RunId::new(), 12);
        let text = id.to_string();
        assert!(text.ends_with(".12"), "{text}");
        assert_eq!(text.parse::<OperationId>(), Ok(id));
    }

    #[test]
    fn run_ids_sort_by_creation() {
        let first = RunId::new();
        let second = RunId::new();
        assert!(first < second);
        assert!(first.to_string() < second.to_string());
    }

    #[test]
    fn malformed_ids_are_refused() {
        assert!("not-a-uuid".parse::<RunId>().is_err());
        assert!("x".parse::<SessionId>().is_err());
        assert!(format!("{}", RunId::new()).parse::<OperationId>().is_err());
        assert!(
            format!("{}.seven", RunId::new())
                .parse::<OperationId>()
                .is_err()
        );
    }

    #[test]
    fn ids_serialize_as_their_text() {
        let id = OperationId::new(RunId::new(), 3);
        let json = serde_json::to_value(id).ok();
        assert_eq!(json, Some(serde_json::Value::String(id.to_string())));
    }
}
