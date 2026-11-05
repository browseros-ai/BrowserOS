//! Records to `claw_sdk_api` models, and request bodies to domain types. This is the only place
//! the wire shapes appear: handlers return generated models, so the JSON cannot drift from the
//! contract. Every function here is pure.

use crate::api_error::ApiError;
use crate::event_log::EventKind;
use crate::limits::SECRET_TTL_SECONDS;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use browseros_core::TabId;
use browseros_trace::{
    Delivery, Effect, EpochMillis, ErrorRecord, OperationId, OperationRow, Page, PageState,
    PrivacyRecord, RunDetail, RunSummary, SecretValue, ShotState, TaskId, TraceTail,
};
use claw_sdk_api as api;
use std::collections::HashMap;
use std::time::Duration;

/// The screenshots a read inlines, base64 JPEGs by operation. Empty unless the request asked
/// for them (`include=screenshots`, `screenshots=`).
#[derive(Default)]
pub(crate) struct Inlined(HashMap<OperationId, String>);

impl Inlined {
    /// Inlines `jpeg` as `op`'s screenshot.
    pub(crate) fn insert(&mut self, op: OperationId, jpeg: &[u8]) {
        self.0.insert(op, STANDARD.encode(jpeg));
    }

    /// `op`'s screenshot as the contract carries it, when inlined.
    fn screenshot(&self, op: OperationId) -> Option<api::Screenshot> {
        self.0
            .get(&op)
            .map(|data| api::Screenshot::new("image/jpeg".to_string(), data.clone()))
    }
}

/// One page of `GET /v1/runs`.
pub(crate) fn run_list(page: &Page<RunSummary>) -> api::RunList {
    api::RunList {
        items: page.items.iter().map(run_summary).collect(),
        next_cursor: page.next_cursor.clone(),
    }
}

pub(crate) fn run_summary(run: &RunSummary) -> api::RunSummary {
    api::RunSummary {
        id: run.id.to_string(),
        task_id: run.task_id.as_ref().map(ToString::to_string),
        tabs: run.tabs.iter().map(|tab| tab.0).collect(),
        status: run.status.as_str().to_string(),
        effects: run.effects.map(|effects| effects.as_str().to_string()),
        error: run
            .error
            .as_ref()
            .map(|error| Box::new(operation_error(error))),
        request_id: run.request_id.as_ref().map(ToString::to_string),
        started_at: run.started_at.millis(),
        ended_at: run.ended_at.map(EpochMillis::millis),
        operation_count: count(run.operation_count),
        complete: run.complete,
        trace_gaps: (run.trace_gaps > 0).then(|| count(run.trace_gaps)),
    }
}

/// One run with every operation; `unknown` lists its writes that were sent and never
/// confirmed, the ones a host must check before repeating anything. `inlined` holds the
/// screenshots `include=screenshots` asked for.
pub(crate) fn run(run: &RunDetail, inlined: &Inlined) -> api::Run {
    let api::RunSummary {
        id,
        task_id,
        tabs,
        status,
        effects,
        error,
        request_id,
        started_at,
        ended_at,
        operation_count,
        complete,
        trace_gaps,
    } = run_summary(&run.summary);
    let unknown = run
        .operations
        .iter()
        .filter(|op| op.effect == Effect::Write && op.delivery == Some(Delivery::Unknown))
        .map(|op| api::UnknownOperation {
            operation_id: op.id.to_string(),
            name: op.name.clone(),
        })
        .collect::<Vec<_>>();
    api::Run {
        id,
        task_id,
        tabs,
        status,
        effects,
        error,
        request_id,
        started_at,
        ended_at,
        operation_count,
        complete,
        trace_gaps,
        host_data: run.host_data.clone(),
        code: run.code.clone(),
        code_withheld: run.code_withheld.clone(),
        result: run.result.clone().map(Some),
        result_truncated: run.result_truncated.then_some(true),
        unknown: (!unknown.is_empty()).then_some(unknown),
        operations: run
            .operations
            .iter()
            .map(|op| operation(op, inlined))
            .collect(),
    }
}

/// One operation. `hasScreenshot` says a JPEG is stored; `screenshot` carries it only when the
/// read inlines it.
pub(crate) fn operation(op: &OperationRow, inlined: &Inlined) -> api::Operation {
    api::Operation {
        id: op.id.to_string(),
        run_id: op.id.run.to_string(),
        parent_id: op.parent.map(|parent| parent.to_string()),
        name: op.name.clone(),
        kind: op.kind.as_str().to_string(),
        attempt: op
            .kind
            .attempt()
            .map(|attempt| i32::try_from(attempt).unwrap_or(i32::MAX)),
        effect: op.effect.as_str().to_string(),
        tab_id: op.tab.map(|tab| tab.0),
        page: page_ref(&op.page).map(Box::new),
        args: op.args.clone().map(Some),
        args_truncated: op.args_truncated.then_some(true),
        result: op.result.clone().map(Some),
        result_truncated: op.result_truncated.then_some(true),
        outcome: op.outcome.map(|outcome| outcome.as_str().to_string()),
        delivery: op.delivery.map(|delivery| delivery.as_str().to_string()),
        error: op
            .error
            .as_ref()
            .map(|error| Box::new(operation_error(error))),
        started_at: op.started_at.millis(),
        sent_at: op.sent_at.map(EpochMillis::millis),
        ended_at: op.ended_at.map(EpochMillis::millis),
        duration_ms: op
            .ended_at
            .map(|ended| ended.millis().saturating_sub(op.started_at.millis())),
        has_screenshot: op.screenshot == Some(ShotState::Stored),
        screenshot: inlined.screenshot(op.id).map(Box::new),
    }
}

/// `GET /v1/trace`. `visuals` says when an operation's screenshot was withheld because its tab
/// was private.
pub(crate) fn trace_tail(tail: &TraceTail, inlined: &Inlined) -> api::TraceTail {
    let withheld = tail
        .operations
        .iter()
        .any(|op| op.screenshot == Some(ShotState::Withheld));
    api::TraceTail {
        runs: tail.runs.iter().map(run_summary).collect(),
        operations: tail
            .operations
            .iter()
            .map(|op| operation(op, inlined))
            .collect(),
        truncated: tail.truncated,
        visuals: if withheld {
            "withheld_private_input"
        } else {
            "available"
        }
        .to_string(),
    }
}

/// A tab's privacy: the record of when and why it went private, or not private.
pub(crate) fn privacy(record: Option<PrivacyRecord>) -> api::Privacy {
    match record {
        Some(record) => api::Privacy {
            private: true,
            since: Some(record.since.millis()),
            reason: Some(record.reason.as_str().to_string()),
        },
        None => api::Privacy::new(false),
    }
}

/// `run.started` or `run.ended`, carrying the run's summary.
pub(crate) fn run_event(kind: EventKind, at: EpochMillis, run: &RunSummary) -> api::Event {
    api::Event {
        task_id: run.task_id.as_ref().map(ToString::to_string),
        run: Some(Box::new(run_summary(run))),
        ..api::Event::new(kind.as_str().to_string(), at.millis())
    }
}

/// `operation.started` or `operation.ended`, carrying the operation and its run's task.
pub(crate) fn operation_event(
    kind: EventKind,
    at: EpochMillis,
    task: Option<&TaskId>,
    op: &OperationRow,
) -> api::Event {
    api::Event {
        task_id: task.map(ToString::to_string),
        tab_id: op.tab.map(|tab| tab.0),
        operation: Some(Box::new(operation(op, &Inlined::default()))),
        ..api::Event::new(kind.as_str().to_string(), at.millis())
    }
}

/// `tab.private`, once the flag is on disk.
pub(crate) fn tab_private_event(tab: TabId, record: PrivacyRecord) -> api::Event {
    api::Event {
        tab_id: Some(tab.0),
        privacy: Some(Box::new(privacy(Some(record)))),
        ..api::Event::new(
            EventKind::TabPrivate.as_str().to_string(),
            record.since.millis(),
        )
    }
}

/// `POST /v1/tabs/{tabId}/private`'s optional body: none, `{}`, or `{"reason": "host"}`.
pub(crate) fn private_request(body: &[u8]) -> Result<(), ApiError> {
    if body.is_empty() {
        return Ok(());
    }
    let request = serde_json::from_slice::<api::MakePrivateRequest>(body)
        .map_err(|_| not_the_body(r#"{"reason"?: "host"}"#))?;
    match request.reason.as_deref() {
        None | Some("host") => Ok(()),
        Some(_) => Err(ApiError::invalid("`reason` must be `host`")),
    }
}

/// `POST /v1/secrets`: the value, already a `SecretValue` (zeroed when dropped), and its TTL.
/// No error quotes the body, and the generated model, which derives `Debug`, is never printed.
pub(crate) fn secret_request(body: &[u8]) -> Result<(SecretValue, Option<Duration>), ApiError> {
    let api::SecretRequest { value, ttl_seconds } = serde_json::from_slice(body)
        .map_err(|_| not_the_body(r#"{"value": string, "ttlSeconds"?: integer}"#))?;
    let value = SecretValue::new(value);
    let ttl = ttl_seconds
        .map(|seconds| {
            u64::try_from(seconds)
                .ok()
                .filter(|seconds| SECRET_TTL_SECONDS.contains(seconds))
                .map(Duration::from_secs)
                .ok_or_else(|| {
                    ApiError::invalid(format!(
                        "`ttlSeconds` must be from {} to {}",
                        SECRET_TTL_SECONDS.start(),
                        SECRET_TTL_SECONDS.end()
                    ))
                })
        })
        .transpose()?;
    Ok((value, ttl))
}

/// A body that does not parse. serde's message is not passed on: it can quote the input.
fn not_the_body(shape: &str) -> ApiError {
    ApiError::invalid(format!("the body must be JSON {shape}"))
}

fn count(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn page_ref(page: &PageState) -> Option<api::PageRef> {
    let PageState {
        page_id,
        tab_id,
        url,
        title,
    } = page;
    if page_id.is_none() && tab_id.is_none() && url.is_none() && title.is_none() {
        return None;
    }
    Some(api::PageRef {
        page_id: page_id.and_then(|id| i32::try_from(id).ok()),
        tab_id: tab_id.map(|tab| tab.0),
        url: url.clone(),
        title: title.clone(),
    })
}

fn operation_error(error: &ErrorRecord) -> api::OperationError {
    api::OperationError {
        owner: error.owner.as_str().to_string(),
        code: error.code.clone(),
        host_code: error.host_code.clone(),
        message: error.message.clone(),
        retryable: error.retryable,
        operation_id: error.operation_id.map(|id| id.to_string()),
    }
}
