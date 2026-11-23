//! An OTLP/HTTP collector in this process: what Neo's push door reaches when a test points
//! `OTEL_EXPORTER_OTLP_ENDPOINT` at it, over TCP or a Unix socket. It keeps every export request
//! and decodes the spans and metric names out of them, in either encoding, and refuses a body
//! whose `Content-Type` is not the one it was started for, so a test also proves the protocol.
//!
//! It serves on a runtime of its own: a test that blocks its thread in a flush (or whose runtime
//! is the one being shut down) still has a collector that answers. It never blocks on that
//! runtime (the sockets are bound with std first) and stops it in the background, so it is
//! started and dropped alike from a plain test thread or from inside Neo's runtime.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Read;
use std::net::TcpListener as StdTcpListener;
use std::os::unix::net::UnixListener as StdUnixListener;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::{CONTENT_ENCODING, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode, Uri};
use flate2::read::GzDecoder;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value;
use opentelemetry_proto::tonic::trace::v1::Span as ProtoSpan;
use prost::Message;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tempfile::TempDir;
use tokio::net::{TcpListener, UnixListener};
use tokio::runtime::{Builder, Runtime};
use tokio::time::{self, Instant};

use crate::test_rig::TestError;

/// How a collector is reached and what it accepts: the transport and the OTLP encoding.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Wire {
    /// `http/protobuf` over loopback TCP.
    ProtobufTcp,
    /// `http/protobuf` over a Unix socket (`http+unix://`).
    ProtobufUnix,
    /// `http/json` over loopback TCP.
    JsonTcp,
}

impl Wire {
    /// `OTEL_EXPORTER_OTLP_PROTOCOL` for this wire.
    pub(crate) const fn protocol(self) -> &'static str {
        match self {
            Self::ProtobufTcp | Self::ProtobufUnix => "http/protobuf",
            Self::JsonTcp => "http/json",
        }
    }

    const fn content_type(self) -> &'static str {
        match self {
            Self::ProtobufTcp | Self::ProtobufUnix => "application/x-protobuf",
            Self::JsonTcp => "application/json",
        }
    }
}

/// One span as the collector decoded it. A plain data bag; ids are lowercase hex, as a
/// `traceparent` spells them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExportedSpan {
    pub(crate) name: String,
    pub(crate) trace_id: String,
    pub(crate) span_id: String,
    /// `None` for a trace's root.
    pub(crate) parent_span_id: Option<String>,
    /// The W3C `tracestate` it carries, `""` when none.
    pub(crate) trace_state: String,
    /// Strings as JSON strings, integers as JSON numbers.
    pub(crate) attributes: BTreeMap<String, Value>,
}

impl ExportedSpan {
    /// The string attribute `key`, if the span has one.
    pub(crate) fn text(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).and_then(Value::as_str)
    }
}

/// One export request as it arrived.
struct Received {
    path: String,
    body: Vec<u8>,
}

#[derive(Clone)]
struct Inbox {
    received: Arc<Mutex<Vec<Received>>>,
    content_type: &'static str,
}

/// The collector. Dropping it stops its runtime and removes its socket's directory.
pub(crate) struct MockCollector {
    base: String,
    wire: Wire,
    received: Arc<Mutex<Vec<Received>>>,
    /// Always `Some` until dropped.
    runtime: Option<Runtime>,
    _socket_dir: Option<TempDir>,
}

impl MockCollector {
    /// Serving, on an ephemeral port or a socket in a fresh temp dir.
    pub(crate) fn start(wire: Wire) -> Result<Self, TestError> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let received = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new().fallback(receive).with_state(Inbox {
            received: Arc::clone(&received),
            content_type: wire.content_type(),
        });
        let (base, socket_dir) = match wire {
            Wire::ProtobufTcp | Wire::JsonTcp => {
                let listener = StdTcpListener::bind("127.0.0.1:0")?;
                listener.set_nonblocking(true)?;
                let base = format!("http://{}", listener.local_addr()?);
                runtime
                    .spawn(async move { axum::serve(TcpListener::from_std(listener)?, app).await });
                (base, None)
            }
            Wire::ProtobufUnix => {
                let dir = tempfile::tempdir()?;
                let socket = dir.path().join("collector.sock");
                let listener = StdUnixListener::bind(&socket)?;
                listener.set_nonblocking(true)?;
                runtime.spawn(
                    async move { axum::serve(UnixListener::from_std(listener)?, app).await },
                );
                (unix_base(&socket), Some(dir))
            }
        };
        Ok(Self {
            base,
            wire,
            received,
            runtime: Some(runtime),
            _socket_dir: socket_dir,
        })
    }

    /// The telemetry variables that point Neo at this collector (`OTEL_EXPORTER_OTLP_ENDPOINT`
    /// is its base: Neo posts to `<base>/v1/traces` and `<base>/v1/metrics`), with both
    /// batching delays cut to `every_ms` so a test sees its spans and metrics promptly.
    pub(crate) fn env(&self, every_ms: u32) -> Vec<(&'static str, String)> {
        vec![
            ("OTEL_EXPORTER_OTLP_ENDPOINT", self.base.clone()),
            (
                "OTEL_EXPORTER_OTLP_PROTOCOL",
                self.wire.protocol().to_owned(),
            ),
            ("OTEL_BSP_SCHEDULE_DELAY", every_ms.to_string()),
            ("OTEL_METRIC_EXPORT_INTERVAL", every_ms.to_string()),
        ]
    }

    /// Every span received so far, in arrival order.
    pub(crate) fn spans(&self) -> Result<Vec<ExportedSpan>, TestError> {
        let mut spans = Vec::new();
        for body in self.bodies("/v1/traces") {
            let request: ExportTraceServiceRequest = self.decode(&body)?;
            spans.extend(
                request
                    .resource_spans
                    .into_iter()
                    .flat_map(|resource| resource.scope_spans)
                    .flat_map(|scope| scope.spans)
                    .map(exported),
            );
        }
        Ok(spans)
    }

    /// The names of every metric received so far, each once.
    pub(crate) fn metric_names(&self) -> Result<Vec<String>, TestError> {
        let mut names = Vec::new();
        for body in self.bodies("/v1/metrics") {
            let request: ExportMetricsServiceRequest = self.decode(&body)?;
            for metric in request
                .resource_metrics
                .into_iter()
                .flat_map(|resource| resource.scope_metrics)
                .flat_map(|scope| scope.metrics)
            {
                if !names.contains(&metric.name) {
                    names.push(metric.name);
                }
            }
        }
        Ok(names)
    }

    /// The spans received once `done` holds for them, polling up to `limit`.
    pub(crate) async fn spans_until(
        &self,
        limit: Duration,
        done: impl Fn(&[ExportedSpan]) -> bool,
    ) -> Result<Vec<ExportedSpan>, TestError> {
        let deadline = Instant::now() + limit;
        loop {
            let spans = self.spans()?;
            if done(&spans) {
                return Ok(spans);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "the collector did not get the spans in {limit:?}: {spans:#?}"
                )
                .into());
            }
            time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The bodies posted to `path`, in arrival order.
    fn bodies(&self, path: &str) -> Vec<Vec<u8>> {
        self.received
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|received| received.path == path)
            .map(|received| received.body.clone())
            .collect()
    }

    /// One OTLP message in this collector's encoding. opentelemetry-proto's `with-serde`
    /// messages read the OTLP JSON spelling (hex ids, integers as strings), so both encodings
    /// land in the same types.
    fn decode<M: Message + Default + DeserializeOwned>(&self, body: &[u8]) -> Result<M, TestError> {
        Ok(match self.wire {
            Wire::ProtobufTcp | Wire::ProtobufUnix => M::decode(body)?,
            Wire::JsonTcp => serde_json::from_slice(body)?,
        })
    }
}

impl Drop for MockCollector {
    fn drop(&mut self) {
        // Not a blocking shutdown: the collector may be dropped inside another runtime.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

/// Keeps a request whose `Content-Type` is the collector's, gunzipped; refuses any other with
/// `415`, so it never decodes.
async fn receive(
    State(inbox): State<Inbox>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let header = |name| headers.get(name).and_then(|value| value.to_str().ok());
    if header(CONTENT_TYPE) != Some(inbox.content_type) {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE;
    }
    let body = match header(CONTENT_ENCODING) {
        None => body.to_vec(),
        Some("gzip") => {
            let mut plain = Vec::new();
            if GzDecoder::new(body.as_ref())
                .read_to_end(&mut plain)
                .is_err()
            {
                return StatusCode::BAD_REQUEST;
            }
            plain
        }
        Some(_) => return StatusCode::UNSUPPORTED_MEDIA_TYPE,
    };
    inbox
        .received
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Received {
            path: uri.path().to_owned(),
            body,
        });
    StatusCode::OK
}

fn exported(span: ProtoSpan) -> ExportedSpan {
    ExportedSpan {
        name: span.name,
        trace_id: hex(&span.trace_id),
        span_id: hex(&span.span_id),
        parent_span_id: (!span.parent_span_id.is_empty()).then(|| hex(&span.parent_span_id)),
        trace_state: span.trace_state,
        attributes: span
            .attributes
            .into_iter()
            .map(|attribute| {
                let value = match attribute.value.and_then(|value| value.value) {
                    Some(any_value::Value::StringValue(text)) => Value::from(text),
                    Some(any_value::Value::IntValue(number)) => Value::from(number),
                    other => Value::from(format!("{other:?}")),
                };
                (attribute.key, value)
            })
            .collect(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        // Writing to a String cannot fail.
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// `http+unix://<socket path, percent-encoded>`: the wire contract's spelling of a Unix socket
/// base, every byte but ASCII letters and digits escaped.
fn unix_base(socket: &Path) -> String {
    let encoded: String = socket
        .to_string_lossy()
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    format!("http+unix://{encoded}")
}
