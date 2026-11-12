//! `GET /v1/events`: server-sent events over the event log.
//!
//! Each event carries `id: <boot>-<seq>`, its type as `event:`, and the contract's `Event` as
//! `data:`; an SSE comment every 15 s keeps an idle stream open. A client starts where its
//! `Last-Event-ID` allows (`event_resume`) and from then on reads the ring after its own seq.
//! When it falls so far behind that the ring dropped entries it never read, it gets
//! `stream.reset` and continues from the newest entry. Filters run per client, on the keys the
//! log stored, with no store read.

use crate::api_error::ApiError;
use crate::event_log::{Entry, EventFilter, EventLog, LogEvent, Read};
use crate::event_resume::Resume;
use crate::limits::SSE_KEEP_ALIVE;
use crate::query;
use crate::service::ServiceState;
use axum::extract::{RawQuery, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::Stream;
use neo_trace::EpochMillis;
use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::watch;

pub(crate) async fn stream_events(
    State(service): State<Arc<ServiceState>>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let filter = query::events(raw.as_deref())?;
    // An id that is not text cannot be one Neo sent; it reads as malformed and resets.
    let last_event_id = headers
        .get("last-event-id")
        .map(|id| id.to_str().unwrap_or_default());
    let start = service.events.resume(last_event_id);
    let stream = events(Arc::clone(&service.events), filter, start);
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(SSE_KEEP_ALIVE)))
}

/// One client's place in the log.
struct Reader {
    log: Arc<EventLog>,
    filter: EventFilter,
    /// The newest seq it has read, sent or filtered out.
    seq: u64,
    ready: VecDeque<Event>,
    wake: watch::Receiver<()>,
}

fn events(
    log: Arc<EventLog>,
    filter: EventFilter,
    start: Resume,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let wake = log.subscribe();
    let mut ready = VecDeque::new();
    let seq = match start {
        Resume::After(seq) => seq,
        Resume::Reset { newest } => {
            ready.push_back(reset(log.boot(), newest));
            newest
        }
    };
    let reader = Reader {
        log,
        filter,
        seq,
        ready,
        wake,
    };
    futures_util::stream::unfold(reader, |mut reader| async move {
        loop {
            if let Some(event) = reader.ready.pop_front() {
                return Some((Ok(event), reader));
            }
            // Marked seen before the ring is read: an entry published after the read still
            // changes the watch, so the wait below cannot sleep through it.
            reader.wake.mark_unchanged();
            match reader.log.read_after(reader.seq) {
                Read::Missed { newest } => {
                    reader.seq = newest;
                    reader.ready.push_back(reset(reader.log.boot(), newest));
                }
                Read::Entries(entries) if entries.is_empty() => {
                    if reader.wake.changed().await.is_err() {
                        return None;
                    }
                }
                Read::Entries(entries) => {
                    for entry in entries {
                        reader.seq = entry.seq;
                        if reader.filter.admits(&entry.event) {
                            reader.ready.push_back(sse(reader.log.boot(), &entry));
                        }
                    }
                }
            }
        }
    })
}

fn sse(boot: &str, entry: &Entry) -> Event {
    Event::default()
        .id(format!("{boot}-{}", entry.seq))
        .event(entry.event.kind.as_str())
        .data(&*entry.event.data)
}

/// A reset for one client. Its id is the newest seq, so a client that reconnects with it
/// resumes from there, after re-reading state.
fn reset(boot: &str, newest: u64) -> Event {
    let event = LogEvent::reset(EpochMillis::now());
    Event::default()
        .id(format!("{boot}-{newest}"))
        .event(event.kind.as_str())
        .data(&*event.data)
}
