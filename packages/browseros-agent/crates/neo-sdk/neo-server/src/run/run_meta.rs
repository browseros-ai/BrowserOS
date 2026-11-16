//! The host's metadata for one `run` call.

use std::fmt;

use neo_trace::{IdError, RequestId, TaskId};
use serde::Deserialize;
use serde_json::Value;

use crate::config::Defaults;
use crate::dispatch::{ErrorCode, OperationError};

/// The `_meta` key a host sends its ids under, and Neo answers the run's id, status and effects
/// under (`neo-sdk-api/openapi.yaml`).
pub(crate) const RUN_META_KEY: &str = "com.browseros.neo/run";

/// `_meta["com.browseros.neo/run"]`, every field optional. Parsed once, at the MCP boundary; a
/// plain data bag after.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RunMeta {
    /// The host's id for this call, the replay key: a repeat of it never runs again.
    pub(crate) request_id: Option<RequestId>,
    /// The host's label for a task's calls: it picks the `page` default and scopes replay.
    pub(crate) task_id: Option<TaskId>,
    /// Stored with the run; Neo never reads it.
    pub(crate) host_data: Option<HostData>,
}

/// The host's own JSON, at most `Defaults::HOST_DATA_MAX_BYTES` serialized. The trace writer
/// cuts host data at 2048 bytes, so a larger value is refused here instead of stored cut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HostData(Value);

/// The wire form. Unknown keys are refused, so a misspelt `requestID` cannot silently turn
/// replay off and let a retry run twice.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WireMeta {
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    host_data: Option<Value>,
}

impl RunMeta {
    /// Reads `_meta["com.browseros.neo/run"]`; absent is empty. Every failure is the host's:
    /// `host / invalid_meta`, naming the field.
    pub(crate) fn parse(meta: Option<&Value>) -> Result<Self, OperationError> {
        let Some(meta) = meta else {
            return Ok(Self::default());
        };
        let wire: WireMeta = serde_path_to_error::deserialize(meta).map_err(|error| {
            let path = error.path().to_string();
            if path == "." {
                invalid(format_args!("{RUN_META_KEY}: {}", error.inner()))
            } else {
                invalid(format_args!("{RUN_META_KEY}.{path}: {}", error.inner()))
            }
        })?;
        let request_id = wire
            .request_id
            .map(|text| RequestId::parse(&text).map_err(|error| host_id("requestId", &error)))
            .transpose()?;
        let task_id = wire
            .task_id
            .map(|text| TaskId::parse(&text).map_err(|error| host_id("taskId", &error)))
            .transpose()?;
        let host_data = wire.host_data.map(HostData::new).transpose()?;
        Ok(Self {
            request_id,
            task_id,
            host_data,
        })
    }
}

impl HostData {
    fn new(value: Value) -> Result<Self, OperationError> {
        let bytes = value.to_string().len();
        if bytes > Defaults::HOST_DATA_MAX_BYTES {
            return Err(invalid(format_args!(
                "{RUN_META_KEY}.hostData is {bytes} bytes as JSON; the limit is {}",
                Defaults::HOST_DATA_MAX_BYTES
            )));
        }
        Ok(Self(value))
    }

    /// The value, for the trace.
    #[must_use]
    pub(crate) fn into_value(self) -> Value {
        self.0
    }
}

fn host_id(field: &str, error: &IdError) -> OperationError {
    invalid(format_args!(
        "{RUN_META_KEY}.{field} must be 1 to {} bytes: {error}",
        Defaults::ID_MAX_BYTES
    ))
}

fn invalid(message: impl fmt::Display) -> OperationError {
    OperationError::host(ErrorCode::INVALID_META, message)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use neo_trace::{ErrorOwner, RequestId, TaskId};
    use serde_json::{Value, json};

    use super::RunMeta;
    use crate::dispatch::ErrorCode;

    fn refused(meta: &Value) -> Result<String, Box<dyn Error>> {
        match RunMeta::parse(Some(meta)) {
            Ok(parsed) => Err(format!("{meta} parsed as {parsed:?}").into()),
            Err(error) => {
                assert_eq!(
                    (error.owner(), error.code()),
                    (ErrorOwner::Host, ErrorCode::INVALID_META),
                    "{meta}: the host sent it, so the host owns it"
                );
                Ok(error.message().to_owned())
            }
        }
    }

    #[test]
    fn every_field_is_optional() -> Result<(), Box<dyn Error>> {
        assert_eq!(RunMeta::parse(None)?, RunMeta::default(), "no _meta at all");
        assert_eq!(RunMeta::parse(Some(&json!({})))?, RunMeta::default());
        assert_eq!(
            RunMeta::parse(Some(
                &json!({ "requestId": null, "taskId": null, "hostData": null })
            ))?,
            RunMeta::default(),
            "null is absent"
        );
        let full = RunMeta::parse(Some(&json!({
            "requestId": "call_81f", "taskId": "task_12", "hostData": { "ownerRun": "r9" },
        })))?;
        assert_eq!(full.request_id, Some(RequestId::parse("call_81f")?));
        assert_eq!(full.task_id, Some(TaskId::parse("task_12")?));
        assert_eq!(
            full.host_data.map(super::HostData::into_value),
            Some(json!({ "ownerRun": "r9" }))
        );
        Ok(())
    }

    #[test]
    fn malformed_meta_is_refused_with_the_field() -> Result<(), Box<dyn Error>> {
        let message = refused(&json!({ "requestID": "x" }))?;
        assert!(
            message.contains("requestID"),
            "a typo would turn replay off silently, so it is named: {message}"
        );
        let long = refused(&json!({ "requestId": "x".repeat(129) }))?;
        assert!(long.contains("requestId must be 1 to 128 bytes"), "{long}");
        assert!(refused(&json!({ "taskId": "" }))?.contains("taskId"));
        assert!(refused(&json!({ "requestId": 7 }))?.contains("requestId"));
        let big = refused(&json!({ "hostData": "x".repeat(2048) }))?;
        assert!(
            big.contains("hostData is 2050 bytes"),
            "refused rather than stored cut: {big}"
        );
        refused(&json!("call_81f"))?;
        Ok(())
    }
}
