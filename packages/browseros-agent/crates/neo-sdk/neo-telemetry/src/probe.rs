//! The read side of [`crate::Telemetry::in_memory`]: the spans the real pipeline ended, as plain
//! values a test can compare. Behind the `test-support` feature.
//!
//! The pipeline behind it is the live one: the same instruments, vocabulary, error-once rule,
//! span builder, privacy dial and Prometheus text. Only the span exporter is replaced, by
//! [`Capture`], a span processor that keeps each sampled span the moment it ends, so a test needs
//! no collector, no flush and no wait. Metrics are read the way the host API reads them, with
//! [`crate::Telemetry::render_prometheus`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use opentelemetry::trace::{SpanId, SpanKind, Status};
use opentelemetry::{Context, Value};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{Span, SpanData, SpanProcessor};

/// What an in-memory handle's spans were: every sampled span ended so far, in the order each
/// ended. Cheap to clone; every clone reads the same spans.
#[derive(Clone, Debug)]
pub struct Probe {
    spans: Arc<Mutex<Vec<SpanData>>>,
}

/// One ended span, as an OTLP exporter would have sent it. A plain data bag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedSpan {
    /// `neo.run`, or an op span's op name (`page.goto`, `unknown`).
    pub name: String,
    /// Its OpenTelemetry span kind.
    pub kind: CapturedKind,
    /// The trace it belongs to, as 32 lowercase hex digits (a `traceparent`'s spelling).
    pub trace_id: String,
    /// Its own id, as 16 lowercase hex digits.
    pub span_id: String,
    /// Its parent's span id, or `None` for a trace's root.
    pub parent_span_id: Option<String>,
    /// Whether the parent is the caller's (a `traceparent`), not a span of this process.
    pub parent_is_remote: bool,
    /// The W3C `tracestate` the span carries (the caller's, inherited), `""` when none.
    pub trace_state: String,
    /// Every attribute, by key.
    pub attributes: BTreeMap<String, SpanValue>,
    /// Its OpenTelemetry status.
    pub status: SpanStatus,
    /// When it started.
    pub start: SystemTime,
    /// When it ended.
    pub end: SystemTime,
}

/// A span's OpenTelemetry kind, as OTLP names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapturedKind {
    /// Serves an inbound call (`neo.run`).
    Server,
    /// In-process work under another span (an op).
    Internal,
    /// An outbound call.
    Client,
    /// Hands work to a queue.
    Producer,
    /// Takes work from a queue.
    Consumer,
}

/// One attribute value. Neo's spans carry only strings and integers; anything else is kept as its
/// text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpanValue {
    /// A string attribute, such as `neo.op.name`.
    Str(String),
    /// An integer attribute, such as `code.line.number` or `neo.tab.id`.
    Int(i64),
}

/// A span's OpenTelemetry status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpanStatus {
    /// Not set: the run or op ended ok.
    Unset,
    /// Set ok (Neo never sets it).
    Ok,
    /// It failed, was cancelled or timed out.
    Error {
        /// The status text: the error code, else how it ended.
        description: String,
    },
}

/// The span processor an in-memory pipeline exports through: keeps each sampled span as it ends.
#[derive(Debug)]
pub(crate) struct Capture {
    spans: Arc<Mutex<Vec<SpanData>>>,
}

impl Probe {
    /// A capture and the probe that reads it back.
    pub(crate) fn new() -> (Capture, Self) {
        let spans = Arc::new(Mutex::new(Vec::new()));
        (
            Capture {
                spans: Arc::clone(&spans),
            },
            Self { spans },
        )
    }

    /// Every sampled span ended so far, in the order each ended. An unsampled span (a caller's
    /// `traceparent` that said "not sampled") never appears, as it would never be exported.
    #[must_use]
    pub fn spans(&self) -> Vec<CapturedSpan> {
        lock(&self.spans).iter().map(captured).collect()
    }
}

impl SpanProcessor for Capture {
    fn on_start(&self, _span: &mut Span, _cx: &Context) {}

    fn on_end(&self, span: SpanData) {
        if span.span_context.is_sampled() {
            lock(&self.spans).push(span);
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        Ok(())
    }
}

// A test that panicked while holding the lock must not hide the spans from the next assertion.
fn lock(spans: &Mutex<Vec<SpanData>>) -> MutexGuard<'_, Vec<SpanData>> {
    spans.lock().unwrap_or_else(PoisonError::into_inner)
}

fn captured(span: &SpanData) -> CapturedSpan {
    CapturedSpan {
        name: span.name.to_string(),
        kind: match span.span_kind {
            SpanKind::Server => CapturedKind::Server,
            SpanKind::Internal => CapturedKind::Internal,
            SpanKind::Client => CapturedKind::Client,
            SpanKind::Producer => CapturedKind::Producer,
            SpanKind::Consumer => CapturedKind::Consumer,
        },
        trace_id: span.span_context.trace_id().to_string(),
        span_id: span.span_context.span_id().to_string(),
        parent_span_id: (span.parent_span_id != SpanId::INVALID)
            .then(|| span.parent_span_id.to_string()),
        parent_is_remote: span.parent_span_is_remote,
        trace_state: span.span_context.trace_state().header(),
        attributes: span
            .attributes
            .iter()
            .map(|attribute| {
                let value = match &attribute.value {
                    Value::I64(number) => SpanValue::Int(*number),
                    Value::String(text) => SpanValue::Str(text.as_str().to_owned()),
                    other => SpanValue::Str(other.to_string()),
                };
                (attribute.key.as_str().to_owned(), value)
            })
            .collect(),
        status: match &span.status {
            Status::Unset => SpanStatus::Unset,
            Status::Ok => SpanStatus::Ok,
            Status::Error { description } => SpanStatus::Error {
                description: description.to_string(),
            },
        },
        start: span.start_time,
        end: span.end_time,
    }
}
