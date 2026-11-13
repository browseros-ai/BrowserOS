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
//! `expired` after retention deletes the file, and 404 when no image was kept.
//!
//! A run's downloads and its reply's blobs are served from the files claw-sdk kept beside the
//! trace when the run ended (`neo_trace::RunOutputs`), streamed. Those are written before the
//! run's reply, unlike its rows, so these reads need no `wait`: a file not kept is a 404.

use crate::api_error::ApiError;
use crate::query::{Fields, Inline};
use crate::service::ServiceState;
use crate::wire::Inlined;
use crate::{query, wire};
use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path, RawQuery, State};
use axum::http::HeaderValue;
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use futures_util::stream;
use neo_sdk_api as api;
use neo_trace::{
    BlobId, KeptFile, OperationRow, RunDetail, RunId, ShotRead, ShotState, TraceError,
    TraceRecordEvent, TraceStore,
};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;

pub(crate) async fn list_runs(
    State(service): State<Arc<ServiceState>>,
    RawQuery(raw): RawQuery,
) -> Result<Json<api::RunList>, ApiError> {
    let query = query::runs(raw.as_deref())?;
    let view = query::view(raw.as_deref())?;
    let page = service.store.runs(query).await?;
    if view.fields == Fields::Full {
        return Ok(Json(wire::run_list(&page)));
    }
    // One metadata query replaces per-run detail reads without loading the payloads the host
    // discards. Keep the captured page and cursor if a run ends or retention removes it mid-read.
    let ids = page.items.iter().map(|run| run.id).collect::<Vec<_>>();
    let operations = service
        .store
        .summary_operations(&ids, view.operations)
        .await?;
    let items = page
        .items
        .iter()
        .map(|summary| {
            wire::audit_run(
                summary,
                operations.get(&summary.id).map_or(&[], Vec::as_slice),
            )
        })
        .collect();
    Ok(Json(api::RunList {
        items,
        next_cursor: page.next_cursor,
    }))
}

pub(crate) async fn get_run(
    State(service): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    RawQuery(raw): RawQuery,
) -> Result<Json<api::Run>, ApiError> {
    let id = query::run_id(&id)?;
    let read = query::run(raw.as_deref())?;
    let mut run = read_run(&service, id, read.wait)
        .await?
        .ok_or_else(|| ApiError::not_found("no run has this id"))?;
    run.operations.retain(|op| read.operations.includes(op));
    let inline = if read.fields == Fields::Full {
        read.inline
    } else {
        Inline::None
    };
    let inlined = inline_screenshots(&service.store, &run.operations, inline).await?;
    Ok(Json(wire::run(&run, &inlined, read.fields)))
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
        ShotRead::Expired => Err(ApiError::expired("retention deleted this screenshot")),
        // Older traces recorded withheld jobs without a JPEG. They read as missing images.
        ShotRead::None | ShotRead::Withheld => {
            Err(ApiError::not_found("this operation has no screenshot"))
        }
    }
}

/// How much of a kept file one chunk of its response reads.
const FILE_CHUNK: usize = 64 * 1024;

/// The characters RFC 8187 lets through unencoded in `filename*` (its `attr-char`).
const ATTR_CHARS: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'!')
    .remove(b'#')
    .remove(b'$')
    .remove(b'&')
    .remove(b'+')
    .remove(b'-')
    .remove(b'.')
    .remove(b'^')
    .remove(b'_')
    .remove(b'`')
    .remove(b'|')
    .remove(b'~');

pub(crate) async fn get_download(
    State(service): State<Arc<ServiceState>>,
    Path((run, index)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let run = query::run_id(&run)?;
    let index: u32 = index
        .parse()
        .map_err(|_| ApiError::not_found("no download has this index"))?;
    let outputs = service.store.outputs();
    let Some(kept) = outputs.open_download(run, index).await? else {
        return Err(ApiError::not_found(
            "this run kept no bytes for this download: it has no such index, or the download had \
             not completed when the run ended (see the run's `downloads`)",
        ));
    };
    let name = outputs
        .downloads(run)
        .await?
        .into_iter()
        .find(|download| download.index == index)
        .map(|download| download.suggested_filename);
    Ok(file_response(kept, &attachment(name.as_deref())))
}

/// The bytes a run's reply names as `{"$neo":"blob","id":…}` because they did not fit inline.
pub(crate) async fn get_blob(
    State(service): State<Arc<ServiceState>>,
    Path((run, blob)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let run = query::run_id(&run)?;
    let blob: BlobId = blob
        .parse()
        .map_err(|_| ApiError::not_found("no blob has this id"))?;
    let Some(kept) = service.store.outputs().open_blob(run, blob).await? else {
        return Err(ApiError::not_found("this run's reply names no such blob"));
    };
    Ok(file_response(kept, "attachment"))
}

/// `Content-Disposition` for a file the host saves, with its name when it has one (RFC 6266).
fn attachment(name: Option<&str>) -> String {
    match name.filter(|name| !name.is_empty()) {
        Some(name) => format!(
            "attachment; filename*=UTF-8''{}",
            utf8_percent_encode(name, ATTR_CHARS)
        ),
        None => "attachment".to_owned(),
    }
}

/// A file a run kept, streamed as `application/octet-stream` in chunks, so a large one never sits
/// in memory whole.
fn file_response(kept: KeptFile, disposition: &str) -> Response {
    let KeptFile { file, size } = kept;
    let chunks = stream::try_unfold(file, |mut file| async move {
        let mut chunk = vec![0; FILE_CHUNK];
        let read = file.read(&mut chunk).await?;
        if read == 0 {
            return Ok::<_, std::io::Error>(None);
        }
        chunk.truncate(read);
        Ok(Some((Bytes::from(chunk), file)))
    });
    let mut response = Body::from_stream(chunks).into_response();
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(CONTENT_LENGTH, HeaderValue::from(size));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(value) = HeaderValue::from_str(disposition) {
        headers.insert(CONTENT_DISPOSITION, value);
    }
    response
}

pub(crate) async fn get_trace(
    State(service): State<Arc<ServiceState>>,
    RawQuery(raw): RawQuery,
) -> Result<Json<api::TraceTail>, ApiError> {
    let read = query::trace(raw.as_deref())?;
    let tail = service.store.trace_tail(read.query).await?;
    let inline = if read.fields == Fields::Full {
        read.inline
    } else {
        Inline::None
    };
    let inlined = inline_screenshots(&service.store, &tail.operations, inline).await?;
    Ok(Json(wire::trace_tail(&tail, &inlined, read.fields)))
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
