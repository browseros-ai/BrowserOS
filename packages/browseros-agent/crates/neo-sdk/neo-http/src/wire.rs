//! Records to `neo_sdk_api` models, and request bodies to domain types. This is the only place
//! the wire shapes appear: handlers return generated models, so the JSON cannot drift from the
//! contract. Every function here is pure.

use crate::api_error::ApiError;
use crate::event_log::EventKind;
use crate::limits::{KEEP_SECONDS, MAX_NAVIGATE_INPUT, VIEWER_SIDE};
use crate::query::Fields;
use crate::setting::{Checked, SETTINGS, Setting, SettingValue};
use crate::settings::{SetValues, Settings};
use crate::tab_keeps::Keep;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use browseros_core::TabId;
use neo_sdk_api as api;
use neo_takeover::{
    BarAction, BarOutcome, Fit, FitMode, FitState, Fitted, Lock, LockTtl, Reads, ReleaseReason,
    ScreenSize, Takeover, TakeoverRequest, TakeoverState, TakeoverTtl, Viewer, ViewerScale,
};
use neo_trace::{
    Delivery, Effect, EpochMillis, ErrorRecord, OperationId, OperationRow, Page, PageState,
    RunDetail, RunDownload, RunSummary, ShotState, TaskId, TraceTail,
};
use std::collections::HashMap;
use std::fmt;
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
    run_summary_fields(run, Fields::Full)
}

fn run_summary_fields(run: &RunSummary, fields: Fields) -> api::RunSummary {
    api::RunSummary {
        values: run.values.clone(),
        id: run.id.to_string(),
        task_id: run.task_id.as_ref().map(ToString::to_string),
        tabs_scope: run.tabs_scope.map(|scope| scope.as_str().to_owned()),
        tabs: run.tabs.iter().map(|tab| tab.0).collect(),
        status: run.status.as_str().to_string(),
        effects: run.effects.map(|effects| effects.as_str().to_string()),
        error: run
            .error
            .as_ref()
            .map(|error| Box::new(operation_error(error, fields))),
        request_id: run.request_id.as_ref().map(ToString::to_string),
        started_at: run.started_at.millis(),
        ended_at: run.ended_at.map(EpochMillis::millis),
        operation_count: count(run.operation_count),
        complete: run.complete,
        trace_gaps: (run.trace_gaps > 0).then(|| count(run.trace_gaps)),
        operations: None,
    }
}

/// The host audit needs runs and their steps in one paged request. The regular run list keeps
/// its existing shape; only this summary view populates the optional operations field.
pub(crate) fn audit_run(summary: &RunSummary, operations: &[OperationRow]) -> api::RunSummary {
    let mut run = run_summary_fields(summary, Fields::Summary);
    run.operations = Some(
        operations
            .iter()
            .map(|op| {
                let mut projected = operation_fields(op, &Inlined::default(), Fields::Summary);
                projected.site = None;
                projected.delivery = None;
                projected
            })
            .collect(),
    );
    run
}

/// One run with the selected operations; `unknown` lists its writes that were sent and never
/// confirmed, the ones a host must check before repeating anything. `inlined` holds the
/// screenshots `include=screenshots` asked for.
pub(crate) fn run(run: &RunDetail, inlined: &Inlined, fields: Fields) -> api::Run {
    let api::RunSummary {
        values,
        id,
        task_id,
        tabs_scope,
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
        operations: _,
    } = run_summary_fields(&run.summary, fields);
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
        values,
        id,
        task_id,
        tabs_scope,
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
        host_data: (fields == Fields::Full)
            .then(|| run.host_data.clone())
            .flatten(),
        code: (fields == Fields::Full).then(|| run.code.clone()).flatten(),
        // Expose the restart marker. Older runs with sensitive input can have absent code.
        code_withheld: run
            .code_withheld
            .as_ref()
            .filter(|reason| fields == Fields::Full && reason.as_str() == "unavailable")
            .cloned(),
        result: (fields == Fields::Full)
            .then(|| run.result.clone().map(Some))
            .flatten(),
        result_truncated: (fields == Fields::Full && run.result_truncated).then_some(true),
        unknown: (!unknown.is_empty()).then_some(unknown),
        operations: run
            .operations
            .iter()
            .map(|op| operation_fields(op, inlined, fields))
            .collect(),
        downloads: (fields == Fields::Full && !run.downloads.is_empty())
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
    operation_fields(op, inlined, Fields::Full)
}

/// Projects at the HTTP boundary, leaving the persisted record and other reads intact.
/// Arguments, results and call logs are omitted. URL and title remain for the host's audit timeline.
fn operation_fields(op: &OperationRow, inlined: &Inlined, fields: Fields) -> api::Operation {
    api::Operation {
        values: op.values.clone(),
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
        args: (fields == Fields::Full)
            .then(|| op.args.clone().map(Some))
            .flatten(),
        args_truncated: (fields == Fields::Full && op.args_truncated).then_some(true),
        result: (fields == Fields::Full)
            .then(|| op.result.clone().map(Some))
            .flatten(),
        result_truncated: (fields == Fields::Full && op.result_truncated).then_some(true),
        outcome: op.outcome.map(|outcome| outcome.as_str().to_string()),
        delivery: op.delivery.map(|delivery| delivery.as_str().to_string()),
        error: op
            .error
            .as_ref()
            .map(|error| Box::new(operation_error(error, fields))),
        notes: (fields == Fields::Full && !op.notes.is_empty()).then(|| op.notes.clone()),
        started_at: op.started_at.millis(),
        sent_at: op.sent_at.map(EpochMillis::millis),
        ended_at: op.ended_at.map(EpochMillis::millis),
        duration_ms: op
            .ended_at
            .map(|ended| ended.millis().saturating_sub(op.started_at.millis())),
        has_screenshot: op.screenshot == Some(ShotState::Stored),
        screenshot: (fields == Fields::Full)
            .then(|| inlined.screenshot(op.id).map(Box::new))
            .flatten(),
        main_world: op.main_world,
        frames_incomplete: op.frames_incomplete,
        internal: op.is_internal(),
    }
}

/// `GET /v1/trace`.
pub(crate) fn trace_tail(tail: &TraceTail, inlined: &Inlined, fields: Fields) -> api::TraceTail {
    api::TraceTail {
        runs: tail
            .runs
            .iter()
            .map(|run| run_summary_fields(run, fields))
            .collect(),
        operations: tail
            .operations
            .iter()
            .map(|op| operation_fields(op, inlined, fields))
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

pub(crate) fn keep(keep: Keep) -> api::TabKeep {
    api::TabKeep {
        expires_at: keep.expires_at.map(EpochMillis::millis),
    }
}

/// An optional object with a bounded integer TTL. Explicit null must not become an unlimited
/// keep through the generated model's `Option`, so inspect presence before deserializing it.
pub(crate) fn keep_request(body: &[u8]) -> Result<Option<Duration>, ApiError> {
    if body.is_empty() {
        return Ok(None);
    }
    let shape = r#"{"ttlSeconds"?: integer}"#;
    let json: serde_json::Value = serde_json::from_slice(body).map_err(|_| not_the_body(shape))?;
    let object = json.as_object().ok_or_else(|| not_the_body(shape))?;
    if object.keys().any(|key| key != "ttlSeconds")
        || object.get("ttlSeconds").is_some_and(|ttl| ttl.is_null())
    {
        return Err(not_the_body(shape));
    }
    let request: api::TabKeepRequest =
        serde_json::from_value(json).map_err(|_| not_the_body(shape))?;
    request
        .ttl_seconds
        .map(|seconds| {
            u64::try_from(seconds)
                .ok()
                .filter(|seconds| KEEP_SECONDS.contains(seconds))
                .map(Duration::from_secs)
                .ok_or_else(|| {
                    ApiError::invalid(format!(
                        "`ttlSeconds` must be from {} to {}",
                        KEEP_SECONDS.start(),
                        KEEP_SECONDS.end(),
                    ))
                })
        })
        .transpose()
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
        // Pure 1x sessions keep their existing wire shape. At 2x the host needs both units
        // to fit locally and to send physical dimensions in a remote framebuffer resize.
        viewer: (fit.viewer.scale == ViewerScale::Two).then(|| {
            let mut viewer = api::ViewerRequest::new(
                fit.viewer.size.width.into(),
                fit.viewer.size.height.into(),
            );
            viewer.scale = Some(fit.viewer.scale.factor().into());
            Box::new(viewer)
        }),
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

/// The fit's logical dimensions and optional 1x or 2x preview scale.
pub(crate) fn viewer_request(body: &[u8]) -> Result<Viewer, ApiError> {
    let input: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| not_the_body(r#"{"width": integer, "height": integer, "scale"?: 1 | 2}"#))?;
    check_scale(&input)?;
    let viewer = serde_json::from_value(input)
        .map_err(|_| not_the_body(r#"{"width": integer, "height": integer, "scale"?: 1 | 2}"#))?;
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
        input_drained: takeover.input_drained,
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
    let input: serde_json::Value = serde_json::from_slice(body).map_err(|_| {
        not_the_body(r#"{"tabId": integer, "viewer"?: {"width": integer, "height": integer, "scale"?: 1 | 2}}"#)
    })?;
    if let Some(viewer) = input.get("viewer") {
        check_scale(viewer)?;
    }
    let api::TakeoverRequest {
        tab_id,
        viewer,
        reads,
        ttl_seconds,
    } = serde_json::from_value(input).map_err(|_| {
        not_the_body(
            r#"{"tabId": integer, "viewer"?: {"width": integer, "height": integer, "scale"?: 1 | 2}, "reads"?: "block" | "allow", "ttlSeconds"?: integer}"#,
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

/// Generated optional fields treat explicit null as absence. Preserve scale omission for
/// refit inheritance, but reject null before decoding into the generated request model.
fn check_scale(viewer: &serde_json::Value) -> Result<(), ApiError> {
    if viewer.get("scale").is_some_and(serde_json::Value::is_null) {
        return Err(ApiError::invalid("`scale` must be the integer 1 or 2"));
    }
    Ok(())
}

/// Validates the request once at the host boundary; Surface resolves omitted-scale inheritance.
fn viewer_size(viewer: api::ViewerRequest) -> Result<Viewer, ApiError> {
    let api::ViewerRequest {
        width,
        height,
        scale,
    } = viewer;
    let scale = match scale {
        None => None,
        Some(1) => Some(ViewerScale::One),
        Some(2) => Some(ViewerScale::Two),
        Some(_) => return Err(ApiError::invalid("`scale` must be the integer 1 or 2")),
    };
    let side = |side: i32| {
        u16::try_from(side)
            .ok()
            .filter(|side| VIEWER_SIDE.contains(side))
    };
    match (side(width), side(height)) {
        (Some(width), Some(height)) => {
            Viewer::new(ScreenSize { width, height }, scale).map_err(ApiError::from)
        }
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

/// A settings body or a saved settings file, checked against the table: the values of the
/// settings it named well, the names it used that are no setting, and each setting it named
/// with a value its rule refuses.
#[derive(Debug, Default)]
pub(crate) struct SettingsBody {
    pub(crate) values: SetValues,
    /// Dotted names the server skipped: a name no setting has, at the level where it stops
    /// being one (`tabs.idleCloseMinutes`, or `files` for a whole unknown group).
    pub(crate) unknown: Vec<String>,
    /// Settings the server has, sent with a name of an open list it does not have (a
    /// `trace.screenshots` mode from a newer server): skipped as if left out. Only the setting
    /// is named, never the value.
    pub(crate) unknown_values: Vec<&'static str>,
    /// One per setting at fault: its name and the rule it broke, never the value.
    pub(crate) faults: Vec<SettingFault>,
}

/// Keeps validation identity separate from its wording. Saved-file loading also handles a
/// fault on an ancestor group, while HTTP renders the same setting-and-rule refusal.
#[derive(Debug)]
pub(crate) struct SettingFault {
    pub(crate) name: String,
    pub(crate) rule: String,
}

impl fmt::Display for SettingFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "`{}` {}", self.name, self.rule)
    }
}

/// `PUT` and `PATCH /v1/settings`: the values to apply, and what was skipped; `faults` is empty.
/// The body is decoded here rather than into the generated `Settings` type, which would drop an
/// unknown name without a word; the host is told what was skipped instead. A refusal names
/// every setting at fault, and then nothing is applied.
pub(crate) fn settings_request(body: &[u8]) -> Result<SettingsBody, ApiError> {
    let shape = r#"{"tabs"?: {"idleCloseSeconds"?: integer}, "trace"?: {…}, "fill"?: {…}}"#;
    let json: serde_json::Value = serde_json::from_slice(body).map_err(|_| not_the_body(shape))?;
    let checked = settings_body(&json).ok_or_else(|| not_the_body(shape))?;
    if !checked.faults.is_empty() {
        let faults: Vec<String> = checked.faults.iter().map(ToString::to_string).collect();
        return Err(ApiError::invalid(faults.join("; ")));
    }
    Ok(checked)
}

/// `json` checked against the table, at any depth; `None` when it is not an object. A group a
/// setting sits under (`tabs`) must hold an object.
pub(crate) fn settings_body(json: &serde_json::Value) -> Option<SettingsBody> {
    let object = json.as_object()?;
    let mut checked = SettingsBody::default();
    check_settings("", object, &mut checked);
    Some(checked)
}

/// One level of [`settings_body`]. It recurses only into groups of the table, so its depth is
/// the table's, whatever the body nests.
fn check_settings(
    prefix: &str,
    object: &serde_json::Map<String, serde_json::Value>,
    checked: &mut SettingsBody,
) {
    for (key, value) in object {
        let name = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if let Some(setting) = Setting::find(&name) {
            match setting.check(value) {
                Checked::Value(value) => {
                    checked.values.insert(setting.name, value);
                }
                Checked::UnknownValue => checked.unknown_values.push(setting.name),
                Checked::Refused(rule) => checked.faults.push(SettingFault {
                    name: setting.name.to_owned(),
                    rule,
                }),
            }
        } else if Setting::is_group(&name) {
            match value.as_object() {
                Some(inner) => check_settings(&name, inner, checked),
                None => checked.faults.push(SettingFault {
                    name,
                    rule: "must be an object of settings".to_owned(),
                }),
            }
        } else {
            checked.unknown.push(name);
        }
    }
}

/// Settings by dotted name, nested as the contract's `Settings` object: what an answer carries
/// and what the file saves, so the file is a `PUT` body.
pub(crate) fn settings_object(
    values: impl IntoIterator<Item = (&'static str, SettingValue)>,
) -> serde_json::Value {
    let mut root = serde_json::Map::new();
    for (name, value) in values {
        let mut level = &mut root;
        let mut parts = name.split('.').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                level.insert(part.to_owned(), Setting::json(&value));
            } else {
                let next = level
                    .entry(part)
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                if !next.is_object() {
                    *next = serde_json::Value::Object(serde_json::Map::new());
                }
                let Some(inner) = next.as_object_mut() else {
                    break;
                };
                level = inner;
            }
        }
    }
    serde_json::Value::Object(root)
}

/// The settings as a host reads them: each at its value, and each default. `skipped` is what a
/// `PUT` or `PATCH` skipped, both lists always present then; a `GET` has neither.
pub(crate) fn settings_state(
    settings: &Settings,
    skipped: Option<SettingsBody>,
) -> Result<api::SettingsState, ApiError> {
    let current = settings_object(
        settings
            .current()
            .into_iter()
            .map(|(setting, value)| (setting.name, value)),
    );
    let defaults = settings_object(
        SETTINGS
            .iter()
            .map(|setting| (setting.name, settings.default_value(setting))),
    );
    // A row the contract's `Settings` schema lacks would be dropped here; a test keeps the
    // table and the schema equal.
    let typed = |json| {
        serde_json::from_value::<api::Settings>(json).map_err(|error| {
            ApiError::internal(format!(
                "the settings table and the contract disagree: {error}"
            ))
        })
    };
    let (unknown, unknown_values) = match skipped {
        Some(SettingsBody {
            unknown,
            unknown_values,
            ..
        }) => (
            Some(unknown),
            Some(unknown_values.into_iter().map(str::to_owned).collect()),
        ),
        None => (None, None),
    };
    Ok(api::SettingsState {
        unknown,
        unknown_values,
        ..api::SettingsState::new(typed(current)?, typed(defaults)?)
    })
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

fn operation_error(error: &ErrorRecord, fields: Fields) -> api::OperationError {
    api::OperationError {
        owner: error.owner.as_str().to_string(),
        code: error.code.clone(),
        host_code: (fields == Fields::Full)
            .then(|| error.host_code.clone())
            .flatten(),
        message: match fields {
            Fields::Full => error.message.clone(),
            // Existing generated clients require a string even when this view omits its text.
            Fields::Summary => String::new(),
        },
        retryable: error.retryable,
        operation_id: error.operation_id.map(|id| id.to_string()),
    }
}
