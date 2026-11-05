//! The record routes: runs, one run, one operation, its screenshot and the trace tail, read from
//! the store.
//!
//! The writer is asynchronous, and screenshots arrive after the operations they show. So a read
//! right after a run's MCP result can see it `running`, not yet written, or ended with its
//! screenshots still being taken. `GET /v1/runs/{runId}?wait=` covers all three: it holds the
//! request until the run is `complete` (ended, and every screenshot job it made answered) or
//! the wait is over, then answers with what the store holds, 404 only when the run never
//! appeared.
//!
//! Screenshots are served from the operation's stored state: `image/jpeg` when stored, 410
//! `withheld` when its tab was private, 410 `expired` when retention deleted the file, and 404
//! when it has none (none was taken, or the job was dropped).

use crate::api_error::ApiError;
use crate::query::Inline;
use crate::service::ServiceState;
use crate::wire::Inlined;
use crate::{query, wire};
use axum::Json;
use axum::extract::{Path, RawQuery, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use browseros_trace::{
    OperationRow, RunDetail, RunId, ShotRead, ShotState, TraceError, TraceRecordEvent, TraceStore,
};
use claw_sdk_api as api;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;

pub(crate) async fn list_runs(
    State(service): State<Arc<ServiceState>>,
    RawQuery(raw): RawQuery,
) -> Result<Json<api::RunList>, ApiError> {
    let query = query::runs(raw.as_deref())?;
    let page = service.store.runs(query).await?;
    Ok(Json(wire::run_list(&page)))
}

pub(crate) async fn get_run(
    State(service): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> Result<Json<api::Run>, ApiError> {
    let id = query::run_id(&id)?;
    let (wait, inline) = query::run(raw.as_deref())?;
    let run = read_run(&service, id, wait)
        .await?
        .ok_or_else(|| ApiError::not_found("no run has this id"))?;
    let inlined = inline_screenshots(&service.store, &run.operations, inline).await?;
    Ok(Json(wire::run(&run, &inlined)))
}

pub(crate) async fn get_operation(
    State(service): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::Operation>, ApiError> {
    let id = query::operation_id(&id)?;
    service
        .store
        .operation(id)
        .await?
        .map(|op| Json(wire::operation(&op, &Inlined::default())))
        .ok_or_else(|| ApiError::not_found("no operation has this id"))
}

pub(crate) async fn get_screenshot(
    State(service): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let id = query::operation_id(&id)?;
    match service.store.screenshot(id).await? {
        ShotRead::Jpeg(jpeg) => Ok((
            [(CONTENT_TYPE, "image/jpeg"), (CACHE_CONTROL, "no-store")],
            jpeg,
        )
            .into_response()),
        ShotRead::Withheld => Err(ApiError::withheld(
            "the tab was private, so no screenshot was kept",
        )),
        ShotRead::Expired => Err(ApiError::expired("retention deleted this screenshot")),
        ShotRead::None => Err(ApiError::not_found("this operation has no screenshot")),
    }
}

pub(crate) async fn get_trace(
    State(service): State<Arc<ServiceState>>,
    RawQuery(raw): RawQuery,
) -> Result<Json<api::TraceTail>, ApiError> {
    let (query, inline) = query::trace(raw.as_deref())?;
    let tail = service.store.trace_tail(query).await?;
    let inlined = inline_screenshots(&service.store, &tail.operations, inline).await?;
    Ok(Json(wire::trace_tail(&tail, &inlined)))
}

/// Reads the screenshots `inline` asks for, base64, from `ops` in order. `Last(n)` keeps the
/// latest `n` operations that have one. A file the sweep deleted since the row was read is left
/// out, so `screenshot` is absent while `hasScreenshot` still says one was stored.
async fn inline_screenshots(
    store: &TraceStore,
    ops: &[OperationRow],
    inline: Inline,
) -> Result<Inlined, TraceError> {
    let stored = ops
        .iter()
        .filter(|op| op.screenshot == Some(ShotState::Stored));
    let ids = match inline {
        Inline::None => return Ok(Inlined::default()),
        Inline::All => stored.map(|op| op.id).collect::<Vec<_>>(),
        Inline::Last(count) => {
            let mut latest = stored.rev().map(|op| op.id).collect::<Vec<_>>();
            latest.truncate(usize::try_from(count).unwrap_or(usize::MAX));
            latest
        }
    };
    let mut inlined = Inlined::default();
    for id in ids {
        if let ShotRead::Jpeg(jpeg) = store.screenshot(id).await? {
            inlined.insert(id, &jpeg);
        }
    }
    Ok(inlined)
}

/// Reads the run, holding up to `wait` until it is complete.
///
/// The receiver is subscribed before the first read, and the writer publishes `RunComplete`
/// only after the commit that sets `complete`. So a completion committed before a read is in
/// that read, and one committed after it is waiting in the receiver: none falls between the
/// two.
async fn read_run(
    service: &ServiceState,
    id: RunId,
    wait: Duration,
) -> Result<Option<RunDetail>, TraceError> {
    let mut events = service.sink.subscribe();
    let deadline = Instant::now() + wait;
    loop {
        let run = service.store.run(id).await?;
        let complete = run.as_ref().is_some_and(|run| run.summary.complete);
        if complete || Instant::now() >= deadline {
            return Ok(run);
        }
        match tokio::time::timeout_at(deadline, completion_of(&mut events, id)).await {
            Ok(true) => {}
            // The wait is over, or the writer is gone: answer with what the store holds now.
            Ok(false) | Err(_) => return service.store.run(id).await,
        }
    }
}

/// Waits until the writer has committed `run`'s completion. A lag counts too: the events it
/// dropped may have held it. False once the writer is gone.
async fn completion_of(events: &mut broadcast::Receiver<TraceRecordEvent>, run: RunId) -> bool {
    loop {
        match events.recv().await {
            Ok(TraceRecordEvent::RunComplete { run: other }) if other == run => return true,
            Ok(_) => {}
            Err(RecvError::Lagged(_)) => return true,
            Err(RecvError::Closed) => return false,
        }
    }
}
