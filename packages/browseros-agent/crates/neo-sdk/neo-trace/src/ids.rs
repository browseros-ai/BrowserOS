//! The ids every record carries. A run id is a UUIDv7, so ids sort by start time and keep index
//! locality. An operation id is its run plus a per-run counter, written `{run}.{seq}`, so an
//! operation id names its run and sorts in start order without a lookup.
//!
//! The host's ids arrive with a run in `_meta["com.browseros.neo/run"]`: a [`RequestId`] for one
//! call and a [`TaskId`] for the calls that belong together. Neo issues neither. It checks only
//! that each is non-empty and at most 128 bytes, then stores it exactly as sent, because the
//! host looks records up by it.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// The longest host id Neo accepts, in bytes.
const MAX_HOST_ID_BYTES: usize = 128;

/// One `run(code)` call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RunId(Uuid);

/// One SDK operation: the run it belongs to and its admission order within that run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OperationId {
    /// The run this operation belongs to.
    pub run: RunId,
    /// Its place in the run's admission order, from 1.
    pub seq: u32,
}

/// The host's own id for one `run` call (`requestId`, e.g. zuzu's tool-call id). A repeat of it
/// never runs again: admission replays the first run instead, per task when the run has one,
/// else across Neo.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(String);

/// The host's label for the runs of one task (`taskId`). Neo never issues or expires it, and an
/// unknown one is fine. It groups runs in the trace, scopes `requestId` replay, and picks the
/// task's `page` default.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(String);

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

impl OperationId {
    /// The `seq`th operation of `run`.
    #[must_use]
    pub fn new(run: RunId, seq: u32) -> Self {
        Self { run, seq }
    }
}

impl RequestId {
    /// Reads a host's request id.
    ///
    /// # Errors
    /// When `text` is empty or longer than 128 bytes.
    pub fn parse(text: &str) -> Result<Self, IdError> {
        host_id("request", text).map(Self)
    }

    /// The id as the host sent it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TaskId {
    /// Reads a host's task id.
    ///
    /// # Errors
    /// When `text` is empty or longer than 128 bytes.
    pub fn parse(text: &str) -> Result<Self, IdError> {
        host_id("task", text).map(Self)
    }

    /// The id as the host sent it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn host_id(kind: &'static str, text: &str) -> Result<String, IdError> {
    if text.is_empty() || text.len() > MAX_HOST_ID_BYTES {
        return Err(IdError {
            kind,
            value: text.to_string(),
        });
    }
    Ok(text.to_string())
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

impl fmt::Display for OperationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.run, self.seq)
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
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

impl FromStr for RequestId {
    type Err = IdError;
    fn from_str(value: &str) -> Result<Self, IdError> {
        Self::parse(value)
    }
}

impl FromStr for TaskId {
    type Err = IdError;
    fn from_str(value: &str) -> Result<Self, IdError> {
        Self::parse(value)
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
string_serde!(OperationId);
string_serde!(RequestId);
string_serde!(TaskId);

#[cfg(test)]
mod tests {
    use super::{OperationId, RequestId, RunId, TaskId};

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

    #[test]
    fn host_ids_are_kept_as_sent_up_to_128_bytes() {
        let longest = "é".repeat(64);
        assert_eq!(
            TaskId::parse(&longest).map(|task| task.as_str().len()),
            Ok(128)
        );
        assert!(TaskId::parse(&format!("{longest}x")).is_err(), "129 bytes");
        assert!(RequestId::parse("").is_err());
        assert!(TaskId::parse("").is_err());
        let request = RequestId::parse(" call 7 ");
        assert_eq!(
            request.as_ref().map(ToString::to_string),
            Ok(" call 7 ".to_string()),
            "not trimmed or rewritten"
        );
    }

    #[test]
    fn host_ids_read_from_json_are_checked() {
        let task = serde_json::from_value::<TaskId>(serde_json::json!("checkout-42"));
        assert_eq!(task.ok(), TaskId::parse("checkout-42").ok());
        assert!(serde_json::from_value::<RequestId>(serde_json::json!("")).is_err());
        let too_long = "r".repeat(129);
        assert!(serde_json::from_value::<RequestId>(serde_json::json!(too_long)).is_err());
    }
}
