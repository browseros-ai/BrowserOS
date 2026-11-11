//! What one `run` call answers.

use neo_trace::{
    Delivery, ErrorOwner, ErrorRecord, OperationId, RequestId, RunEffects, RunId, RunStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dispatch::{ErrorCode, OpFailure, OpLine, OperationError, RunCtx, RunEnd, StopReason};
use crate::runtime::{LogLine, ScriptEnd, ScriptOutcome};

/// What one `run` call answers: the MCP layer renders it, and the trace keeps
/// it as the run's replay, so a repeat of its `requestId` gets back exactly this, `id`
/// included, and renders the same way. A plain data bag.
///
/// `status` is `ok` exactly when `error` is `None`; `value` is the script's return value then,
/// and `null` otherwise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunReply {
    /// Neo's id for the run: what the host looks it up by in the trace.
    pub(crate) id: RunId,
    /// How the run ended.
    pub(crate) status: ReplyStatus,
    /// Whether it changed anything, from its writes' delivery.
    pub(crate) effects: ReplyEffects,
    /// The script's return value.
    #[serde(default)]
    pub(crate) value: Value,
    /// Why it did not return a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<ReplyError>,
    /// Its console lines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) logs: Vec<LogLine>,
    /// Console lines past the run's limits.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(crate) logs_dropped: usize,
    /// Writes that were sent and never confirmed: look at the page before repeating them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) unknown: Vec<OpNote>,
    /// Ops still running when the script returned; they were cut short.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) not_awaited: Vec<OpNote>,
}

/// How a run ended, as the caller sees it: the trace's `RunStatus` without the states a reply
/// never has (`running`, `interrupted`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplyStatus {
    /// The script returned.
    Ok,
    /// The script failed, or the call was refused.
    Error,
    /// The caller left, or Neo shut down.
    Cancelled,
    /// The run's budget ran out.
    Timeout,
}

/// The trace's `RunEffects` on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplyEffects {
    /// No write was sent.
    None,
    /// Every write that was sent got a definite answer.
    Done,
    /// A write was sent and never confirmed.
    Unknown,
}

/// The trace's `Delivery` on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplyDelivery {
    /// Nothing reached the page.
    NotSent,
    /// The page answered.
    Done,
    /// Sent and never confirmed.
    Unknown,
}

/// The run's error, in the shape of the trace's `ErrorRecord`. A plain data bag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReplyError {
    /// Who must change to fix it.
    pub(crate) owner: ErrorOwner,
    /// The stable code.
    pub(crate) code: String,
    /// The human message, masked.
    pub(crate) message: String,
    /// Whether repeating the whole run is safe: it changed nothing.
    pub(crate) retryable: bool,
    /// For an op's error that escaped the script: the op.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) operation_id: Option<OperationId>,
    /// For an op's error that escaped the script: whether its write reached the page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) delivery: Option<ReplyDelivery>,
}

/// One op in the reply's lines. A plain data bag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpNote {
    /// The op, `{run}.{seq}`.
    pub(crate) operation_id: OperationId,
    /// Its method.
    pub(crate) name: String,
    /// Where the script called it, `run:LINE:COL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) site: Option<String>,
}

impl RunReply {
    /// The reply for a run that went through its script. A stop the run did not cause
    /// itself (the deadline, the caller, shutdown, a crash) decides the status, whatever the
    /// script did after it; otherwise the script's outcome does.
    #[must_use]
    pub(crate) fn build(run: &RunCtx, end: ScriptEnd) -> Self {
        let report = run.report();
        let effects = ReplyEffects::from(report.effects);
        // Repeating a run is safe only when it changed nothing.
        let retryable = effects == ReplyEffects::None;
        let stopped = run.stop_reason().and_then(status_of_stop);
        let (status, value, error) = match (stopped, end.outcome) {
            (Some((status, reason)), _) => (
                status,
                Value::Null,
                Some(ReplyError::from_error(&reason.error(), retryable)),
            ),
            (None, ScriptOutcome::Returned(value)) => (ReplyStatus::Ok, value, None),
            (None, ScriptOutcome::OperationFailed(failure)) => (
                ReplyStatus::Error,
                Value::Null,
                Some(ReplyError::from_failure(&failure, retryable)),
            ),
            (None, ScriptOutcome::Failed(error)) => (
                ReplyStatus::Error,
                Value::Null,
                Some(ReplyError::from_error(&error, retryable)),
            ),
            (None, ScriptOutcome::Stopped) => (
                ReplyStatus::Error,
                Value::Null,
                Some(ReplyError::from_error(
                    &OperationError::neo(
                        ErrorCode::INTERNAL_ERROR,
                        "the script stopped, and the run had no reason to stop it",
                    ),
                    retryable,
                )),
            ),
        };
        let (logs, logs_dropped) = end.logs.into_parts();
        Self {
            id: run.id(),
            status,
            effects,
            value,
            error,
            logs,
            logs_dropped,
            unknown: report.unknown.iter().map(OpNote::from).collect(),
            not_awaited: report.not_awaited.iter().map(OpNote::from).collect(),
        }
    }

    /// A call refused before it ran: nothing happened, so repeating it is safe.
    #[must_use]
    pub(crate) fn refused(id: RunId, error: &OperationError) -> Self {
        Self::failed(id, ReplyEffects::None, ReplyError::from_error(error, true))
    }

    /// The run's own task ended without a reply (it panicked). Its ops may have sent anything.
    #[must_use]
    pub(crate) fn crashed(id: RunId) -> Self {
        let error = OperationError::neo(
            ErrorCode::INTERNAL_ERROR,
            "the run failed inside Neo; its trace says what it sent",
        );
        Self::failed(
            id,
            ReplyEffects::Unknown,
            ReplyError::from_error(&error, false),
        )
    }

    /// A repeat of a `requestId` whose replay window has passed, as call `id` (D14). It is this
    /// call's own refusal: nothing ran and nothing was sent, while the first run's own id and
    /// effects stay in the trace, which the message points to. Not retryable, because a repeat
    /// gets the same refusal and the first run may have changed the page.
    #[must_use]
    pub(crate) fn duplicate(id: RunId, first: RunId) -> Self {
        let error = OperationError::host(
            ErrorCode::DUPLICATE_REQUEST,
            format_args!(
                "this requestId already ran as run {first}, whose result is no longer kept, so \
                 this call ran nothing; look that run up in the trace instead of repeating it"
            ),
        );
        let error = ReplyError {
            delivery: Some(ReplyDelivery::NotSent),
            ..ReplyError::from_error(&error, false)
        };
        Self::failed(id, ReplyEffects::None, error)
    }

    /// The trace's end record, with this reply as the replay when a repeat of `request` could
    /// ask for it.
    #[must_use]
    pub(crate) fn trace_end(&self, request: Option<&RequestId>) -> RunEnd {
        RunEnd {
            status: self.status.trace_status(),
            error: self.error.as_ref().map(ReplyError::record),
            result: match self.status {
                ReplyStatus::Ok => Some(self.value.clone()),
                ReplyStatus::Error | ReplyStatus::Cancelled | ReplyStatus::Timeout => None,
            },
            replay: request.and_then(|_| serde_json::to_value(self).ok()),
        }
    }

    fn failed(id: RunId, effects: ReplyEffects, error: ReplyError) -> Self {
        Self {
            id,
            status: ReplyStatus::Error,
            effects,
            value: Value::Null,
            error: Some(error),
            logs: Vec::new(),
            logs_dropped: 0,
            unknown: Vec::new(),
            not_awaited: Vec::new(),
        }
    }
}

/// The status a stop decides, with the stop: every reason but the script's own return.
fn status_of_stop(reason: StopReason) -> Option<(ReplyStatus, StopReason)> {
    let status = match reason {
        StopReason::Deadline => ReplyStatus::Timeout,
        StopReason::CallerLeft | StopReason::Shutdown => ReplyStatus::Cancelled,
        StopReason::Crashed => ReplyStatus::Error,
        StopReason::ScriptReturned => return None,
    };
    Some((status, reason))
}

impl ReplyStatus {
    /// The trace's status for `RunEnded`.
    #[must_use]
    pub(crate) fn trace_status(self) -> RunStatus {
        match self {
            Self::Ok => RunStatus::Ok,
            Self::Error => RunStatus::Error,
            Self::Cancelled => RunStatus::Cancelled,
            Self::Timeout => RunStatus::Timeout,
        }
    }
}

impl From<RunEffects> for ReplyEffects {
    fn from(effects: RunEffects) -> Self {
        match effects {
            RunEffects::None => Self::None,
            RunEffects::Done => Self::Done,
            RunEffects::Unknown => Self::Unknown,
        }
    }
}

impl ReplyDelivery {
    /// The wire name, as the trace stores it.
    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NotSent => "not_sent",
            Self::Done => "done",
            Self::Unknown => "unknown",
        }
    }
}

impl From<Delivery> for ReplyDelivery {
    fn from(delivery: Delivery) -> Self {
        match delivery {
            Delivery::NotSent => Self::NotSent,
            Delivery::Done => Self::Done,
            Delivery::Unknown => Self::Unknown,
        }
    }
}

impl ReplyError {
    fn from_error(error: &OperationError, retryable: bool) -> Self {
        Self {
            owner: error.owner(),
            code: error.code().as_str().to_owned(),
            message: error.message().to_owned(),
            retryable,
            operation_id: None,
            delivery: None,
        }
    }

    /// An op's failure that escaped the script: its owner and code, the op it came from, and
    /// what reached the page.
    fn from_failure(failure: &OpFailure, retryable: bool) -> Self {
        Self {
            operation_id: Some(failure.operation),
            delivery: Some(ReplyDelivery::from(failure.delivery)),
            ..Self::from_error(&failure.error, retryable)
        }
    }

    /// The trace's form.
    #[must_use]
    pub(crate) fn record(&self) -> ErrorRecord {
        ErrorRecord {
            owner: self.owner,
            code: self.code.clone(),
            message: self.message.clone(),
            host_code: None,
            retryable: self.retryable,
            operation_id: self.operation_id,
        }
    }
}

impl From<&OpLine> for OpNote {
    fn from(line: &OpLine) -> Self {
        Self {
            operation_id: line.operation,
            name: line.name.to_owned(),
            site: line.site.map(|site| site.to_string()),
        }
    }
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if hands the field by reference"
)]
fn is_zero(count: &usize) -> bool {
    *count == 0
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use neo_trace::{ErrorOwner, OperationId, RunId};

    use super::{OpNote, ReplyDelivery, ReplyEffects, ReplyStatus, RunReply};
    use crate::dispatch::{ErrorCode, OperationError};
    use crate::runtime::{LogLevel, LogLine};

    #[test]
    fn a_stored_replay_reads_back_as_the_same_reply() -> Result<(), Box<dyn Error>> {
        let id = RunId::new();
        let mut reply = RunReply::duplicate(id, RunId::new());
        reply.logs = vec![LogLine {
            level: LogLevel::Warn,
            text: "careful".to_owned(),
        }];
        reply.logs_dropped = 3;
        reply.unknown = vec![OpNote {
            operation_id: OperationId::new(id, 4),
            name: "page.goto".to_owned(),
            site: Some("run:3:7".to_owned()),
        }];
        if let Some(error) = &mut reply.error {
            error.delivery = Some(ReplyDelivery::Unknown);
            error.operation_id = Some(OperationId::new(id, 4));
        }
        let stored = serde_json::to_value(&reply)?;
        assert_eq!(
            serde_json::from_value::<RunReply>(stored)?,
            reply,
            "a repeat of the requestId renders exactly what the first call got"
        );
        Ok(())
    }

    #[test]
    fn a_refused_call_changed_nothing_and_says_so() {
        let error = OperationError::neo(ErrorCode::BROWSER_UNAVAILABLE, "not connected yet");
        let reply = RunReply::refused(RunId::new(), &error);
        assert_eq!(reply.status, ReplyStatus::Error);
        assert_eq!(reply.effects, ReplyEffects::None);
        let error = reply.error.as_ref();
        assert_eq!(error.map(|error| error.owner), Some(ErrorOwner::Neo));
        assert_eq!(
            error.map(|error| error.retryable),
            Some(true),
            "nothing ran, so repeating it is safe"
        );
    }
}
