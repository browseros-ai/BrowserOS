//! The host's metadata for one `run` call.

use std::fmt;

use neo_telemetry::TraceContext;
use neo_trace::{IdError, RequestId, RunAccess, TaskId};
use serde::{Deserialize, Deserializer};
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
    /// The host's restriction on this run's operations, independent of task reach.
    pub(crate) access: Option<RunAccess>,
    /// The caller's W3C trace context, which the run's `neo.run` span joins: the request's
    /// `traceparent` / `tracestate` HTTP headers when their `traceparent` is valid, else this
    /// `_meta`'s pair when its is, else `None`, a trace of the run's own. Never a refusal: a
    /// malformed value only costs the caller its link to the run.
    pub(crate) trace: Option<TraceContext>,
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
    #[serde(default, deserialize_with = "present_access")]
    access: Option<RunAccess>,
    /// W3C trace context for a host that cannot set HTTP headers; the headers win.
    #[serde(default, deserialize_with = "trace_text")]
    traceparent: Option<String>,
    #[serde(default, deserialize_with = "trace_text")]
    tracestate: Option<String>,
}

// Only a missing key means ordinary access. Deserializing Option directly would also accept
// explicit null, silently dropping a host's requested restriction instead of refusing it.
fn present_access<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<RunAccess>, D::Error> {
    RunAccess::deserialize(deserializer).map(Some)
}

// Trace context never refuses a run (wire contract, "Trace context"): a value that is not a
// string is ignored like a malformed one, and the run starts a trace of its own. Only the key
// itself is checked, by `deny_unknown_fields`.
fn trace_text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Value::deserialize(deserializer).map(|value| match value {
        Value::String(text) => Some(text),
        _ => None,
    })
}

impl RunMeta {
    /// Reads `_meta["com.browseros.neo/run"]`; absent is empty. Every failure is the host's:
    /// `host / invalid_meta`, naming the field. `header` is the request's own
    /// `(traceparent, tracestate)` headers, which win over `_meta`'s pair
    /// (`TraceContext::resolve`, the one place that rule lives).
    pub(crate) fn parse(
        meta: Option<&Value>,
        header: Option<(&str, Option<&str>)>,
    ) -> Result<Self, OperationError> {
        let Some(meta) = meta else {
            return Ok(Self {
                trace: TraceContext::resolve(header, None),
                ..Self::default()
            });
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
        let meta_pair = wire
            .traceparent
            .as_deref()
            .map(|traceparent| (traceparent, wire.tracestate.as_deref()));
        Ok(Self {
            request_id,
            task_id,
            host_data,
            access: wire.access,
            trace: TraceContext::resolve(header, meta_pair),
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

    use neo_telemetry::TraceContext;
    use neo_trace::{ErrorOwner, RequestId, RunAccess, TaskId};
    use serde::Deserialize;
    use serde_json::{Value, json};

    use super::RunMeta;
    use crate::dispatch::ErrorCode;

    fn refused(meta: &Value) -> Result<String, Box<dyn Error>> {
        match RunMeta::parse(Some(meta), None) {
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
        assert_eq!(
            RunMeta::parse(None, None)?,
            RunMeta::default(),
            "no _meta at all"
        );
        assert_eq!(RunMeta::parse(Some(&json!({})), None)?, RunMeta::default());
        assert_eq!(
            RunMeta::parse(
                Some(&json!({ "requestId": null, "taskId": null, "hostData": null })),
                None
            )?,
            RunMeta::default(),
            "null is absent"
        );
        let full = RunMeta::parse(
            Some(&json!({
                "requestId": "call_81f", "taskId": "task_12", "hostData": { "ownerRun": "r9" },
            })),
            None,
        )?;
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

    #[test]
    fn access_is_absent_or_exactly_read_never_null() -> Result<(), Box<dyn Error>> {
        assert_eq!(RunMeta::parse(None, None)?.access, None);
        assert_eq!(
            RunMeta::parse(Some(&json!({ "access": "read" })), None)?.access,
            Some(RunAccess::Read)
        );
        for access in [
            Value::Null,
            json!(false),
            json!(0),
            json!({}),
            json!([]),
            json!("all"),
            json!("write"),
            json!("Read"),
            json!("read "),
        ] {
            let message = refused(&json!({ "access": access }))?;
            assert!(
                message.contains(".access"),
                "the host sees the field: {message}"
            );
        }
        Ok(())
    }

    #[test]
    fn an_older_metadata_parser_refuses_access_instead_of_ignoring_it() -> Result<(), Box<dyn Error>>
    {
        // The old server has exactly these keys and the same unknown-field refusal. This is
        // the compatibility fence: a host cannot unknowingly get unrestricted execution.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, rename_all = "camelCase")]
        struct OlderMeta {
            request_id: Option<String>,
            task_id: Option<String>,
            host_data: Option<Value>,
        }
        let ordinary: OlderMeta = serde_json::from_value(json!({}))?;
        assert!(
            ordinary.request_id.is_none()
                && ordinary.task_id.is_none()
                && ordinary.host_data.is_none()
        );
        let error = serde_json::from_value::<OlderMeta>(json!({ "access": "read" }))
            .err()
            .ok_or("the older parser must refuse the unknown key")?;
        assert!(
            error.to_string().contains("unknown field `access`"),
            "{error}"
        );
        Ok(())
    }

    /// Two callers' traces (W3C `traceparent`): one sent as the request's HTTP header, one in
    /// `_meta`.
    const HEADER: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    const META: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

    #[test]
    fn trace_context_comes_from_meta_when_no_header_carries_one() -> Result<(), Box<dyn Error>> {
        let meta = json!({ "traceparent": META, "tracestate": "host=1" });
        assert_eq!(
            RunMeta::parse(Some(&meta), None)?.trace,
            TraceContext::from_w3c(META, Some("host=1")),
            "a host that cannot set headers links its run through _meta"
        );
        assert_eq!(
            RunMeta::parse(Some(&json!({ "traceparent": META })), None)?.trace,
            TraceContext::from_w3c(META, None),
            "tracestate is optional"
        );
        assert_eq!(
            RunMeta::parse(Some(&json!({ "tracestate": "host=1" })), None)?.trace,
            None,
            "a tracestate travels only with its own traceparent"
        );
        assert_eq!(
            RunMeta::parse(None, Some((HEADER, Some("edge=1"))))?.trace,
            TraceContext::from_w3c(HEADER, Some("edge=1")),
            "a call with no _meta at all still joins its header's trace"
        );
        Ok(())
    }

    #[test]
    fn the_header_wins_over_meta() -> Result<(), Box<dyn Error>> {
        let meta = json!({ "traceparent": META, "tracestate": "host=1" });
        assert_eq!(
            RunMeta::parse(Some(&meta), Some((HEADER, Some("edge=1"))))?.trace,
            TraceContext::from_w3c(HEADER, Some("edge=1")),
            "the W3C header is trace context's own carrier and _meta only the fallback for a \
             host that cannot set one; the header's pair is taken whole, tracestate included"
        );
        Ok(())
    }

    #[test]
    fn an_invalid_trace_value_is_ignored_and_the_run_still_runs() -> Result<(), Box<dyn Error>> {
        for value in [
            json!("garbage"),
            json!(""),
            json!("00-00000000000000000000000000000000-00f067aa0ba902b7-01"),
            json!(7),
            json!(null),
            json!([META]),
            json!({ "traceparent": META }),
        ] {
            let meta = json!({ "requestId": "call_1", "traceparent": value, "tracestate": 7 });
            let parsed = RunMeta::parse(Some(&meta), None)?;
            assert_eq!(
                parsed.trace, None,
                "{value} is no usable traceparent: a trace of the run's own, never a refusal"
            );
            assert_eq!(
                parsed.request_id,
                Some(RequestId::parse("call_1")?),
                "the rest of _meta still applies"
            );
        }
        assert_eq!(
            RunMeta::parse(
                Some(&json!({ "traceparent": META })),
                Some(("garbage", Some("edge=1")))
            )?
            .trace,
            TraceContext::from_w3c(META, None),
            "a malformed header falls back to _meta's pair, without the header's tracestate"
        );
        assert_eq!(
            RunMeta::parse(None, Some(("garbage", None)))?.trace,
            None,
            "nothing usable at all: a trace of the run's own"
        );
        Ok(())
    }

    #[test]
    fn unknown_keys_beside_trace_context_are_still_refused() -> Result<(), Box<dyn Error>> {
        for key in [
            "traceParent",
            "trace_parent",
            "traceState",
            "traceid",
            "baggage",
        ] {
            let message = refused(&json!({ "traceparent": META, key: "x" }))?;
            assert!(
                message.contains(key),
                "a misspelt key is named, never silently dropped: {message}"
            );
        }
        Ok(())
    }
}
