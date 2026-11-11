//! Records to `neo_sdk_api` models, and request bodies to domain types. This is the only place
//! the wire shapes appear: handlers return generated models, so the JSON cannot drift from the
//! contract. Every function here is pure.

use crate::api_error::ApiError;
use crate::event_log::EventKind;
use crate::limits::{MAX_NAVIGATE_INPUT, VIEWER_SIDE};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use browseros_core::TabId;
use neo_sdk_api as api;
use neo_takeover::{
    BarAction, BarOutcome, Fit, FitMode, FitState, Fitted, Lock, LockTtl, Reads, ReleaseReason,
    ScreenSize, Takeover, TakeoverRequest, TakeoverState, TakeoverTtl,
};
use neo_trace::{
    Delivery, Effect, EpochMillis, ErrorRecord, OperationId, OperationRow, Page, PageState,
    RunDetail, RunDownload, RunSummary, ShotState, TaskId, TraceTail,
};
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
        // Expose the restart marker. Older runs with sensitive input can have absent code.
        code_withheld: run
            .code_withheld
            .as_ref()
            .filter(|reason| reason.as_str() == "unavailable")
            .cloned(),
        result: run.result.clone().map(Some),
        result_truncated: run.result_truncated.then_some(true),
        unknown: (!unknown.is_empty()).then_some(unknown),
        operations: run
            .operations
            .iter()
            .map(|op| operation(op, inlined))
            .collect(),
        downloads: (!run.downloads.is_empty())
            .then(|| run.downloads.iter().map(run_download).collect()),
    }
}

/// One of a run's downloads.
fn run_download(download: &RunDownload) -> api::RunDownload {
    api::RunDownload {
        index: i32::try_from(download.index).unwrap_or(i32::MAX),
        url: download.url.clone(),
        suggested_filename: download.suggested_filename.clone(),
        path: download.path.clone(),
        size: download
            .size
            .map(|size| i64::try_from(size).unwrap_or(i64::MAX)),
        state: download.state.as_str().to_owned(),
        failure: download.failure.clone(),
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
        site: op.site.map(|site| {
            Box::new(api::OperationSite {
                line: i64::from(site.line),
                column: i64::from(site.column),
            })
        }),
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
        notes: (!op.notes.is_empty()).then(|| op.notes.clone()),
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

/// `GET /v1/trace`.
pub(crate) fn trace_tail(tail: &TraceTail, inlined: &Inlined) -> api::TraceTail {
    api::TraceTail {
        runs: tail.runs.iter().map(run_summary).collect(),
        operations: tail
            .operations
            .iter()
            .map(|op| operation(op, inlined))
            .collect(),
        truncated: tail.truncated,
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

/// `tab.closed`, once the tab's locks ended and its private flag was forgotten.
pub(crate) fn tab_closed_event(tab: TabId, at: EpochMillis) -> api::Event {
    api::Event {
        tab_id: Some(tab.0),
        ..api::Event::new(EventKind::TabClosed.as_str().to_string(), at.millis())
    }
}

/// A lock, held, or ended with `releasedAt`: by the host, its takeover, its TTL or its tab
/// closing. The contract has no field for the cause.
pub(crate) fn lock(lock: &Lock) -> api::Lock {
    api::Lock {
        id: lock.id.to_string(),
        tab_id: lock.tab.0,
        reads: match lock.reads {
            Reads::Block => "block",
            Reads::Allow => "allow",
        }
        .to_string(),
        settled: lock.settled,
        created_at: lock.created_at.millis(),
        expires_at: lock.expires_at.millis(),
        released_at: lock.ended.map(|ended| ended.at.millis()),
    }
}

/// `lock.state`: the lock as it stands, at the moment it was taken or ended.
pub(crate) fn lock_event(lock: &Lock) -> api::Event {
    let at = lock.ended.map_or(lock.created_at, |ended| ended.at);
    api::Event {
        tab_id: Some(lock.tab.0),
        lock: Some(Box::new(self::lock(lock))),
        ..api::Event::new(EventKind::LockState.as_str().to_string(), at.millis())
    }
}

/// `POST /v1/locks`: the tab, `reads` (`block` when absent), and a TTL of 1 s to 20 minutes.
pub(crate) fn lock_request(body: &[u8]) -> Result<(TabId, Reads, LockTtl), ApiError> {
    let api::LockRequest {
        tab_id,
        reads,
        ttl_seconds,
    } = serde_json::from_slice(body).map_err(|_| {
        not_the_body(r#"{"tabId": integer, "reads"?: "block" | "allow", "ttlSeconds": integer}"#)
    })?;
    let reads = self::reads(reads.as_deref())?;
    let ttl = u64::try_from(ttl_seconds)
        .ok()
        .map(Duration::from_secs)
        .and_then(LockTtl::new)
        .ok_or_else(|| {
            ApiError::invalid(format!(
                "`ttlSeconds` must be from {} to {}",
                LockTtl::MIN.as_secs(),
                LockTtl::MAX.as_secs()
            ))
        })?;
    Ok((TabId(tab_id), reads, ttl))
}

/// A fit as the contract has it. `fit` is `fitted` or `partial`, never `pending`: the fit
/// route answers once the fit ran.
pub(crate) fn fit(fit: &Fit) -> api::Fit {
    api::Fit {
        id: fit.id.to_string(),
        tab_id: fit.tab.0,
        screen: Box::new(api::ViewerSize::new(
            fit.screen.width.into(),
            fit.screen.height.into(),
        )),
        mode: Some(
            match fit.mode {
                FitMode::Mobile => "mobile",
                FitMode::Desktop => "desktop",
            }
            .to_string(),
        ),
        fit: match fit.fitted {
            Fitted::Full => "fitted",
            Fitted::Partial => "partial",
        }
        .to_string(),
        keyboard: fit.keyboard.as_ref().map(|keyboard| {
            Box::new(api::FitKeyboard::new(
                keyboard.screen.width.into(),
                keyboard.screen.height.into(),
                keyboard.at.millis(),
                keyboard
                    .boxes
                    .iter()
                    .map(|bounds| bounds.to_vec())
                    .collect(),
            ))
        }),
        browser: fit.bar.as_ref().map(|bar| {
            Box::new(api::FitBrowserBar::new(
                bar.url.clone(),
                bar.back,
                bar.forward,
            ))
        }),
        popups: Some(fit.popups.iter().map(|target| target.0.clone()).collect()),
        created_at: fit.created_at.millis(),
        restored_at: match fit.state {
            FitState::Active => None,
            FitState::Restored(at) => Some(at.millis()),
        },
    }
}

/// `POST /v1/tabs/{tabId}/fit`: the viewer's `{width, height}`, each 100 to 4096 px.
pub(crate) fn viewer_request(body: &[u8]) -> Result<ScreenSize, ApiError> {
    let viewer = serde_json::from_slice(body)
        .map_err(|_| not_the_body(r#"{"width": integer, "height": integer}"#))?;
    viewer_size(viewer)
}

/// A takeover as the contract has it. `display` is the control display a host opens for the
/// owner, bound to the takeover with `release=`, so the takeover's release closes it.
pub(crate) fn takeover(takeover: &Takeover) -> api::Takeover {
    let (state, release_reason, released_at) = match takeover.state {
        TakeoverState::Active => ("active", None, None),
        TakeoverState::Released { reason, at } => (
            "released",
            Some(match reason {
                ReleaseReason::Done => "done",
                ReleaseReason::Idle => "idle",
                ReleaseReason::Host => "host",
                ReleaseReason::Expired => "expired",
                ReleaseReason::TabClosed => "tab_closed",
            }),
            Some(at.millis()),
        ),
    };
    api::Takeover {
        id: takeover.id.to_string(),
        tab_id: takeover.tab.0,
        state: state.to_string(),
        release_reason: release_reason.map(str::to_string),
        lock_id: takeover.lock.to_string(),
        fit_id: takeover.fit.map(|fit| fit.to_string()),
        display: format!("/v1/display?mode=control&release={}", takeover.id),
        created_at: takeover.created_at.millis(),
        released_at,
        expires_at: takeover.expires_at.millis(),
    }
}

/// `takeover.state`: the takeover as it stands, when it started and when it was released.
pub(crate) fn takeover_event(takeover: &Takeover) -> api::Event {
    let at = match takeover.state {
        TakeoverState::Active => takeover.created_at,
        TakeoverState::Released { at, .. } => at,
    };
    api::Event {
        tab_id: Some(takeover.tab.0),
        takeover: Some(Box::new(self::takeover(takeover))),
        ..api::Event::new(EventKind::TakeoverState.as_str().to_string(), at.millis())
    }
}

/// `POST /v1/takeovers`: the tab, an optional viewer (each side 100 to 4096 px), `reads`
/// (`block` when absent) and a TTL of 60 s to 20 minutes (20 minutes when absent).
pub(crate) fn takeover_request(body: &[u8]) -> Result<TakeoverRequest, ApiError> {
    let api::TakeoverRequest {
        tab_id,
        viewer,
        reads,
        ttl_seconds,
    } = serde_json::from_slice(body).map_err(|_| {
        not_the_body(
            r#"{"tabId": integer, "viewer"?: {"width": integer, "height": integer}, "reads"?: "block" | "allow", "ttlSeconds"?: integer}"#,
        )
    })?;
    let ttl = match ttl_seconds {
        None => Some(TakeoverTtl::MAX),
        Some(seconds) => u64::try_from(seconds).ok().map(Duration::from_secs),
    }
    .and_then(TakeoverTtl::new)
    .ok_or_else(|| {
        ApiError::invalid(format!(
            "`ttlSeconds` must be from {} to {}",
            TakeoverTtl::MIN.as_secs(),
            TakeoverTtl::MAX.as_secs()
        ))
    })?;
    Ok(TakeoverRequest {
        tab: TabId(tab_id),
        viewer: viewer.map(|viewer| viewer_size(*viewer)).transpose()?,
        reads: self::reads(reads.as_deref())?,
        ttl,
    })
}

/// `POST /v1/takeovers/{takeoverId}/release`'s optional body: none, `{}`, or `{"reason": ..}`
/// with `done` (when absent), `idle` or `host`. `expired` and `tab_closed` are Neo's own reasons.
pub(crate) fn release_request(body: &[u8]) -> Result<ReleaseReason, ApiError> {
    if body.is_empty() {
        return Ok(ReleaseReason::Done);
    }
    let api::ReleaseRequest { reason } = serde_json::from_slice(body)
        .map_err(|_| not_the_body(r#"{"reason"?: "done" | "idle" | "host"}"#))?;
    match reason.as_deref() {
        None | Some("done") => Ok(ReleaseReason::Done),
        Some("idle") => Ok(ReleaseReason::Idle),
        Some("host") => Ok(ReleaseReason::Host),
        Some(_) => Err(ApiError::invalid("`reason` must be done, idle or host")),
    }
}

/// A lock's `reads`, as a lock and a takeover take it: `block` when absent.
fn reads(text: Option<&str>) -> Result<Reads, ApiError> {
    match text {
        None | Some("block") => Ok(Reads::Block),
        Some("allow") => Ok(Reads::Allow),
        Some(_) => Err(ApiError::invalid("`reads` must be block or allow")),
    }
}

/// A viewer's size, each side 100 to 4096 px, as a fit and a takeover take it.
fn viewer_size(viewer: api::ViewerSize) -> Result<ScreenSize, ApiError> {
    let api::ViewerSize { width, height } = viewer;
    let side = |side: i32| {
        u16::try_from(side)
            .ok()
            .filter(|side| VIEWER_SIDE.contains(side))
    };
    match (side(width), side(height)) {
        (Some(width), Some(height)) => Ok(ScreenSize { width, height }),
        _ => Err(ApiError::invalid(format!(
            "`width` and `height` must each be from {} to {}",
            VIEWER_SIDE.start(),
            VIEWER_SIDE.end()
        ))),
    }
}

/// `POST /v1/fits/{fitId}/navigate`: `back`, `forward`, `reload`, or `open` with what the owner
/// typed, at most 2000 characters. The typed text is never quoted back in an error.
pub(crate) fn navigate_request(body: &[u8]) -> Result<BarAction, ApiError> {
    let api::NavigateRequest { action, input } = serde_json::from_slice(body).map_err(|_| {
        not_the_body(r#"{"action": "back" | "forward" | "reload" | "open", "input"?: string}"#)
    })?;
    match (action.as_str(), input) {
        ("back", _) => Ok(BarAction::Back),
        ("forward", _) => Ok(BarAction::Forward),
        ("reload", _) => Ok(BarAction::Reload),
        ("open", Some(input)) if input.chars().count() <= MAX_NAVIGATE_INPUT => {
            Ok(BarAction::Open(input))
        }
        ("open", Some(_)) => Err(ApiError::invalid(format!(
            "`input` is at most {MAX_NAVIGATE_INPUT} characters"
        ))),
        ("open", None) => Err(ApiError::invalid(
            "`open` needs `input`: what the owner typed",
        )),
        _ => Err(ApiError::invalid(
            "`action` must be back, forward, reload or open",
        )),
    }
}

/// What the owner's browser bar did, with the sentence the owner page shows when it did not
/// move.
pub(crate) fn navigate_result(outcome: BarOutcome) -> api::NavigateResult {
    api::NavigateResult {
        status: match outcome {
            BarOutcome::Completed => "completed",
            BarOutcome::Refused => "refused",
            BarOutcome::Failed => "failed",
        }
        .to_string(),
        notice: outcome.notice().map(str::to_string),
    }
}

/// A body that does not parse. serde's message is not passed on: it can quote the input.
fn not_the_body(shape: &str) -> ApiError {
    ApiError::invalid(format!("the body must be JSON {shape}"))
}

fn count(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn page_ref(page: &PageState) -> Option<api::PageRef> {
    let PageState { tab_id, url, title } = page;
    if tab_id.is_none() && url.is_none() && title.is_none() {
        return None;
    }
    Some(api::PageRef {
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
