//! The push door: OTLP/HTTP exporters, the resource and the sampler, from [`OtlpConfig`].
//!
//! Owned by the OTLP work. This is the seam with the pipeline (`pipeline.rs`): its provider
//! builders call [`push`], [`resource`] and [`sampler`] and plug in whatever they return, so
//! neither side edits the other's file.
//!
//! ```text
//!  OtlpConfig ─ push ─┬─ Push::spans    BatchSpanProcessor ─ otlp SpanExporter ──┐ an ExportClient
//!                     ├─ Push::metrics  PeriodicReader ───── otlp MetricExporter ─┤ each
//!                     └─ Push::transport  I/O thread + shutdown deadline ◀────────┘ (transport.rs)
//! ```
//!
//! What the pipeline does with a [`Push`]: build the tracer provider only when `spans` is `Some`
//! (spans leave Neo only over OTLP); give `metrics` to the meter provider beside the scrape's
//! reader; keep `transport` as long as the providers. To shut down within a budget, call
//! `transport.arm_deadline(budget)` first, then the providers' shutdowns on a plain thread: every
//! export, in flight or final, then returns within the budget, collector up, down or hung.
//!
//! Callers never wait on push. An ended span goes into a bounded queue ([`MAX_QUEUED_SPANS`])
//! that drops on overflow (the SDK warns once, at the first drop, and counts drops at shutdown);
//! a metric write is an in-memory aggregate the reader collects on its own thread.

use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry_otlp::{
    Compression, ExporterBuildError, MetricExporter, Protocol, RetryPolicy, SpanExporter,
    WithExportConfig, WithHttpConfig,
};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::{PeriodicReader, Temporality};
use opentelemetry_sdk::trace::{BatchConfigBuilder, BatchSpanProcessor, Sampler};

use crate::config::{SamplerChoice, SignalPush};
use crate::transport::{Signal, Transport};
use crate::{OtlpConfig, OtlpProtocol, TelemetryError};

/// Ended spans that may wait for export. The next one is dropped, never waited for.
const MAX_QUEUED_SPANS: usize = 2048;

/// Spans in one export request.
const SPANS_PER_EXPORT: usize = 512;

/// Everything push needs, owned together: the transport lives exactly as long as the processor
/// and the reader that send through it, so dropping the pipeline cannot leave one without the
/// other.
#[derive(Debug)]
pub(crate) struct Push {
    /// The trace push: a bounded, non-blocking batch processor over an OTLP/HTTP span
    /// exporter; `None` when no traces endpoint is configured.
    pub(crate) spans: Option<BatchSpanProcessor>,
    /// The metric push: a periodic reader (cumulative temporality) over an OTLP/HTTP metric
    /// exporter; `None` when no metrics endpoint is configured.
    pub(crate) metrics: Option<PeriodicReader<MetricExporter>>,
    /// The exporters' I/O thread, and the deadline shutdown arms.
    pub(crate) transport: Transport,
}

/// The push door for `config`, or `None` when no OTLP endpoint is set (the default): then no
/// thread starts and no socket opens. Starts no connection either way.
///
/// Every exporter setting is explicit, because opentelemetry-otlp reads `OTEL_EXPORTER_OTLP_*`
/// from the process environment for anything left unset. Its retries are off: they sleep the
/// SDK's thread and ignore time already spent; the transport retries spans inside its limit
/// instead. Protobuf bodies are gzipped; JSON bodies go plain, for Neo Viewer's sake (the
/// exporter still gzips them if the process environment sets `OTEL_EXPORTER_OTLP_COMPRESSION`).
pub(crate) fn push(config: &OtlpConfig) -> Result<Option<Push>, TelemetryError> {
    if config.traces.is_none() && config.metrics.is_none() {
        return Ok(None);
    }
    let transport = Transport::start()?;
    let spans = match &config.traces {
        Some(traces) => Some(span_processor(traces, config.batch_delay, &transport)?),
        None => None,
    };
    let metrics = match &config.metrics {
        Some(metrics) => Some(metric_reader(metrics, config.metric_interval, &transport)?),
        None => None,
    };
    Ok(Some(Push {
        spans,
        metrics,
        transport,
    }))
}

fn span_processor(
    traces: &SignalPush,
    batch_delay: Duration,
    transport: &Transport,
) -> Result<BatchSpanProcessor, TelemetryError> {
    let exporter = SpanExporter::builder()
        .with_http()
        .with_protocol(protocol(traces.protocol))
        .with_endpoint(traces.endpoint.exporter_url())
        .with_timeout(traces.timeout)
        .with_retry_policy(RetryPolicy::disabled())
        .with_http_client(transport.client(Signal::Traces, traces)?);
    let exporter = match compression(traces.protocol) {
        Some(compression) => exporter.with_compression(compression),
        None => exporter,
    };
    let exporter = exporter
        .build()
        .map_err(|error| not_built("span exporter", &error))?;
    // Queue, batch size and delay are set here, so the SDK's own reading of `OTEL_BSP_*`
    // decides none of them. It also reads `OTEL_BSP_EXPORT_TIMEOUT` and
    // `OTEL_BSP_MAX_CONCURRENT_EXPORTS`, which this thread-based processor ignores: it exports
    // one batch at a time, each bounded by the transport.
    let batches = BatchConfigBuilder::default()
        .with_max_queue_size(MAX_QUEUED_SPANS)
        .with_max_export_batch_size(SPANS_PER_EXPORT)
        .with_scheduled_delay(batch_delay)
        .build();
    Ok(BatchSpanProcessor::builder(exporter)
        .with_batch_config(batches)
        .build())
}

fn metric_reader(
    metrics: &SignalPush,
    interval: Duration,
    transport: &Transport,
) -> Result<PeriodicReader<MetricExporter>, TelemetryError> {
    let exporter = MetricExporter::builder()
        .with_http()
        .with_protocol(protocol(metrics.protocol))
        .with_endpoint(metrics.endpoint.exporter_url())
        .with_timeout(metrics.timeout)
        .with_retry_policy(RetryPolicy::disabled())
        .with_http_client(transport.client(Signal::Metrics, metrics)?)
        .with_temporality(Temporality::Cumulative);
    let exporter = match compression(metrics.protocol) {
        Some(compression) => exporter.with_compression(compression),
        None => exporter,
    };
    let exporter = exporter
        .build()
        .map_err(|error| not_built("metric exporter", &error))?;
    Ok(PeriodicReader::builder(exporter)
        .with_interval(interval)
        .build())
}

fn protocol(protocol: OtlpProtocol) -> Protocol {
    match protocol {
        OtlpProtocol::HttpProtobuf => Protocol::HttpBinary,
        OtlpProtocol::HttpJson => Protocol::HttpJson,
    }
}

/// Gzip for protobuf, the production default (design §4); none for JSON.
fn compression(protocol: OtlpProtocol) -> Option<Compression> {
    match protocol {
        OtlpProtocol::HttpProtobuf => Some(Compression::Gzip),
        OtlpProtocol::HttpJson => None,
    }
}

fn not_built(what: &str, error: &ExporterBuildError) -> TelemetryError {
    TelemetryError::Pipeline {
        stage: "start",
        reason: format!("the OTLP {what} could not be built: {error}"),
    }
}

/// The resource both providers share: `service.name` (`OTEL_SERVICE_NAME`, default
/// `browseros-neo`), then `OTEL_RESOURCE_ATTRIBUTES`, applied last so it wins. Built from the
/// config alone: `Resource::builder` would read the process environment behind its back.
pub(crate) fn resource(config: &OtlpConfig) -> Resource {
    let attributes = config
        .resource_attributes
        .iter()
        .map(|attribute| KeyValue::new(attribute.key.clone(), attribute.value.clone()));
    Resource::builder_empty()
        .with_service_name(config.service_name.clone())
        .with_attributes(attributes)
        .build()
}

/// The tracer provider's sampler (`OTEL_TRACES_SAMPLER` / `_ARG`, default
/// `parentbased_always_on`).
pub(crate) fn sampler(config: &OtlpConfig) -> Sampler {
    match config.sampler {
        SamplerChoice::ParentBasedAlwaysOn => Sampler::ParentBased(Box::new(Sampler::AlwaysOn)),
        SamplerChoice::ParentBasedAlwaysOff => Sampler::ParentBased(Box::new(Sampler::AlwaysOff)),
        SamplerChoice::ParentBasedTraceIdRatio(ratio) => {
            Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(ratio.get())))
        }
        SamplerChoice::AlwaysOn => Sampler::AlwaysOn,
        SamplerChoice::AlwaysOff => Sampler::AlwaysOff,
        SamplerChoice::TraceIdRatio(ratio) => Sampler::TraceIdRatioBased(ratio.get()),
    }
}

#[cfg(test)]
mod tests {
    //! The push door end to end: exporters built by [`push`], providers built here the way the
    //! pipeline builds them, and an OTLP/HTTP collector in this process (axum, on a loopback port
    //! or a Unix socket in a temp directory) that decodes what arrives.

    use std::collections::HashMap;
    use std::error::Error;
    use std::io::Read;
    use std::net::TcpListener as StdTcpListener;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::time::{Duration, Instant};
    use std::{future, thread};

    use axum::Router;
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode, Uri};
    use flate2::read::GzDecoder;
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
    use opentelemetry::{Key, Value};
    use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
    use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
    use opentelemetry_proto::tonic::common::v1::{KeyValue as ProtoKeyValue, any_value};
    use opentelemetry_proto::tonic::metrics::v1::{
        AggregationTemporality, metric, number_data_point,
    };
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    use prost::Message as _;
    use serde_json::Value as Json;
    use tempfile::TempDir;
    use tokio::net::{TcpListener, UnixListener};
    use tokio::runtime::{Builder, Runtime};
    use tokio::sync::watch;

    use super::*;

    /// Spans the queue tests end, about three times what the queue holds.
    const ENDED: usize = 6_000;

    /// How the in-process collector answers.
    #[derive(Clone)]
    enum Answer {
        /// `200` and an empty body, as a healthy collector does.
        Ok,
        /// Never.
        Hang,
        /// `200` once the sender turns it true; until then the request waits.
        HoldUntil(watch::Receiver<bool>),
        /// `503` while the count runs down to 0, then `200`.
        Unavailable(Arc<AtomicUsize>),
    }

    #[derive(Clone, Copy, Debug)]
    enum Wire {
        Tcp,
        Unix,
    }

    /// One export request as the collector received it.
    struct Received {
        path: String,
        headers: HeaderMap,
        body: Bytes,
    }

    #[derive(Clone)]
    struct Handler {
        received: Sender<Received>,
        answer: Answer,
    }

    async fn receive(
        State(handler): State<Handler>,
        uri: Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> StatusCode {
        // A send error means the test is over and no longer listening.
        let _ = handler.received.send(Received {
            path: uri.path().to_owned(),
            headers,
            body,
        });
        match handler.answer {
            Answer::Ok => StatusCode::OK,
            Answer::Hang => future::pending::<StatusCode>().await,
            Answer::HoldUntil(mut release) => {
                let _ = release.wait_for(|released| *released).await;
                StatusCode::OK
            }
            Answer::Unavailable(failures) => {
                let failing = failures
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                        left.checked_sub(1)
                    })
                    .is_ok();
                if failing {
                    StatusCode::SERVICE_UNAVAILABLE
                } else {
                    StatusCode::OK
                }
            }
        }
    }

    /// An OTLP/HTTP collector in this process that keeps every request it receives. It runs on a
    /// runtime of its own, so the test thread can block in the SDK's flushes while it serves.
    struct MockCollector {
        /// `OTEL_EXPORTER_OTLP_ENDPOINT` for it.
        base: String,
        received: Receiver<Received>,
        _runtime: Runtime,
        _socket_dir: Option<TempDir>,
    }

    impl MockCollector {
        fn start(wire: Wire, answer: Answer) -> Result<Self, Box<dyn Error>> {
            let runtime = Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?;
            let (sender, received) = mpsc::channel();
            let app = Router::new().fallback(receive).with_state(Handler {
                received: sender,
                answer,
            });
            let (base, socket_dir) = match wire {
                Wire::Tcp => {
                    let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0"))?;
                    let base = format!("http://{}", listener.local_addr()?);
                    runtime.spawn(async move { axum::serve(listener, app).await });
                    (base, None)
                }
                Wire::Unix => {
                    let dir = tempfile::tempdir()?;
                    let socket = dir.path().join("collector.sock");
                    let listener = runtime.block_on(async { UnixListener::bind(&socket) })?;
                    runtime.spawn(async move { axum::serve(listener, app).await });
                    (unix_url(&socket), Some(dir))
                }
            };
            Ok(Self {
                base,
                received,
                _runtime: runtime,
                _socket_dir: socket_dir,
            })
        }

        /// The next request, waiting up to 5 s for it.
        fn next(&self) -> Result<Received, Box<dyn Error>> {
            Ok(self.received.recv_timeout(Duration::from_secs(5))?)
        }
    }

    fn unix_url(socket: &Path) -> String {
        let socket = socket.to_string_lossy();
        format!(
            "http+unix://{}",
            utf8_percent_encode(&socket, NON_ALPHANUMERIC)
        )
    }

    /// An endpoint nothing listens on: a loopback port just closed, or a socket not there.
    fn nobody_at(wire: Wire, dir: &TempDir) -> Result<String, Box<dyn Error>> {
        Ok(match wire {
            Wire::Tcp => format!(
                "http://{}",
                StdTcpListener::bind("127.0.0.1:0")?.local_addr()?
            ),
            Wire::Unix => unix_url(&dir.path().join("absent.sock")),
        })
    }

    fn config(vars: &[(&str, &str)]) -> Result<OtlpConfig, TelemetryError> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        OtlpConfig::from_env(&|name| vars.get(name).cloned())
    }

    /// The providers the pipeline builds from a [`Push`] (pipeline.rs), built here so these
    /// tests stand on their own.
    struct Providers {
        tracers: SdkTracerProvider,
        meters: SdkMeterProvider,
        transport: Transport,
    }

    impl Providers {
        fn new(config: &OtlpConfig) -> Result<Self, Box<dyn Error>> {
            let Push {
                spans,
                metrics,
                transport,
            } = push(config)?.ok_or("an endpoint must turn push on")?;
            let mut tracers = SdkTracerProvider::builder()
                .with_resource(resource(config))
                .with_sampler(sampler(config));
            if let Some(spans) = spans {
                tracers = tracers.with_span_processor(spans);
            }
            let mut meters = SdkMeterProvider::builder().with_resource(resource(config));
            if let Some(metrics) = metrics {
                meters = meters.with_reader(metrics);
            }
            Ok(Self {
                tracers: tracers.build(),
                meters: meters.build(),
                transport,
            })
        }

        /// One `neo.run` span and `neo_runs_total` +3, as a finished run records them.
        fn record_a_run(&self) {
            let mut span = self.tracers.tracer("neo-telemetry").start("neo.run");
            span.set_attribute(KeyValue::new("neo.run.id", "run-1"));
            span.end();
            self.meters
                .meter("neo-telemetry")
                .u64_counter("neo_runs_total")
                .build()
                .add(3, &[KeyValue::new("status", "ok")]);
        }

        /// What the pipeline's shutdown does: arm the transport's deadline, then shut both
        /// providers down on a plain thread. Returns how long that thread took, waiting at most
        /// ten budgets, so a broken bound fails the test instead of hanging it.
        fn shut_down(self, budget: Duration) -> Result<Duration, Box<dyn Error>> {
            let Self {
                tracers,
                meters,
                transport,
            } = self;
            let start = Instant::now();
            transport.arm_deadline(budget);
            let (done, finished) = mpsc::channel();
            thread::spawn(move || {
                // Failed exports were the transport's to report; only the time matters here.
                let _ = tracers.shutdown_with_timeout(budget);
                let _ = meters.shutdown_with_timeout(budget);
                let _ = done.send(start.elapsed());
                drop(transport);
            });
            Ok(finished.recv_timeout(budget * 10)?)
        }
    }

    /// The body, gunzipped when it says it is gzip.
    fn decoded(received: &Received) -> Result<Vec<u8>, Box<dyn Error>> {
        match header(received, "content-encoding") {
            None => Ok(received.body.to_vec()),
            Some("gzip") => {
                let mut body = Vec::new();
                GzDecoder::new(received.body.as_ref()).read_to_end(&mut body)?;
                Ok(body)
            }
            Some(other) => Err(format!("unexpected content-encoding `{other}`").into()),
        }
    }

    fn header<'a>(received: &'a Received, name: &str) -> Option<&'a str> {
        received
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
    }

    fn string_attribute<'a>(attributes: &'a [ProtoKeyValue], key: &str) -> Option<&'a str> {
        let attribute = attributes.iter().find(|attribute| attribute.key == key)?;
        match attribute.value.as_ref()?.value.as_ref()? {
            any_value::Value::StringValue(value) => Some(value.as_str()),
            _ => None,
        }
    }

    fn span_count(received: &Received) -> Result<usize, Box<dyn Error>> {
        let request = ExportTraceServiceRequest::decode(decoded(received)?.as_slice())?;
        Ok(request
            .resource_spans
            .iter()
            .flat_map(|resource| &resource.scope_spans)
            .map(|scope| scope.spans.len())
            .sum())
    }

    fn check_protobuf(traces: &Received, metrics: &Received) -> Result<(), Box<dyn Error>> {
        let request = ExportTraceServiceRequest::decode(decoded(traces)?.as_slice())?;
        let resource_spans = request.resource_spans.first().ok_or("one resource")?;
        let resource = resource_spans.resource.as_ref().ok_or("a resource")?;
        assert_eq!(
            string_attribute(&resource.attributes, "service.name"),
            Some("neo-under-test")
        );
        assert_eq!(
            string_attribute(&resource.attributes, "deployment.environment"),
            Some("test")
        );
        let spans: Vec<&str> = resource_spans
            .scope_spans
            .iter()
            .flat_map(|scope| &scope.spans)
            .map(|span| span.name.as_str())
            .collect();
        assert_eq!(spans, ["neo.run"]);

        let request = ExportMetricsServiceRequest::decode(decoded(metrics)?.as_slice())?;
        let metric = request
            .resource_metrics
            .iter()
            .flat_map(|resource| &resource.scope_metrics)
            .flat_map(|scope| &scope.metrics)
            .find(|metric| metric.name == "neo_runs_total")
            .ok_or("neo_runs_total must arrive")?;
        let Some(metric::Data::Sum(sum)) = &metric.data else {
            return Err("a counter arrives as a sum".into());
        };
        assert_eq!(
            sum.aggregation_temporality,
            i32::from(AggregationTemporality::Cumulative),
            "OTLP metrics are cumulative (wire contract)"
        );
        assert_eq!(
            sum.data_points.first().and_then(|point| point.value),
            Some(number_data_point::Value::AsInt(3))
        );
        Ok(())
    }

    /// The OTLP JSON encoding as the wire contract pins it: hex ids, 64-bit integers as strings,
    /// enums as integers. Read raw, the way Neo Viewer reads it without a protobuf library.
    fn check_json(traces: &Received, metrics: &Received) -> Result<(), Box<dyn Error>> {
        let body = decoded(traces)?;
        let _: ExportTraceServiceRequest = serde_json::from_slice(&body)?;
        let json: Json = serde_json::from_slice(&body)?;
        let resource_spans = &json["resourceSpans"][0];
        let service = resource_spans["resource"]["attributes"]
            .as_array()
            .and_then(|attributes| {
                attributes
                    .iter()
                    .find(|attribute| attribute["key"] == "service.name")
            })
            .ok_or("the resource names the service")?;
        assert_eq!(service["value"]["stringValue"], "neo-under-test");
        let span = &resource_spans["scopeSpans"][0]["spans"][0];
        assert_eq!(span["name"], "neo.run");
        let hex = |value: &Json, digits: usize| {
            value
                .as_str()
                .is_some_and(|id| id.len() == digits && id.chars().all(|c| c.is_ascii_hexdigit()))
        };
        assert!(hex(&span["traceId"], 32), "trace ids are hex: {span}");
        assert!(hex(&span["spanId"], 16), "span ids are hex: {span}");
        assert!(
            span["startTimeUnixNano"].is_string(),
            "64-bit integers are strings: {span}"
        );
        assert!(span["kind"].is_u64(), "enums are integers: {span}");

        let body = decoded(metrics)?;
        let _: ExportMetricsServiceRequest = serde_json::from_slice(&body)?;
        let json: Json = serde_json::from_slice(&body)?;
        let metric = json["resourceMetrics"][0]["scopeMetrics"][0]["metrics"]
            .as_array()
            .and_then(|metrics| {
                metrics
                    .iter()
                    .find(|metric| metric["name"] == "neo_runs_total")
            })
            .ok_or("neo_runs_total must arrive")?;
        assert_eq!(
            metric["sum"]["aggregationTemporality"], 2,
            "cumulative, as an integer"
        );
        assert_eq!(
            metric["sum"]["dataPoints"][0]["asInt"], "3",
            "64-bit integers are strings"
        );
        Ok(())
    }

    /// One span and one counter go to a collector over `wire` in `protocol`, and what arrives is
    /// checked decoded: paths, content type, encoding, headers, resource and payload.
    fn both_signals_arrive(wire: Wire, protocol: OtlpProtocol) -> Result<(), Box<dyn Error>> {
        let collector = MockCollector::start(wire, Answer::Ok)?;
        let providers = Providers::new(&config(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", collector.base.as_str()),
            ("OTEL_EXPORTER_OTLP_PROTOCOL", protocol.as_str()),
            (
                "OTEL_EXPORTER_OTLP_HEADERS",
                "authorization=Bearer%20s3cret,x-neo-test=yes",
            ),
            ("OTEL_SERVICE_NAME", "neo-under-test"),
            ("OTEL_RESOURCE_ATTRIBUTES", "deployment.environment=test"),
        ])?)?;
        providers.record_a_run();
        providers.tracers.force_flush()?;
        providers.meters.force_flush()?;
        let mut arrived = HashMap::new();
        for _ in 0..2 {
            let received = collector.next()?;
            arrived.insert(received.path.clone(), received);
        }
        let traces = arrived
            .remove("/v1/traces")
            .ok_or("spans go to <base>/v1/traces")?;
        let metrics = arrived
            .remove("/v1/metrics")
            .ok_or("metrics go to <base>/v1/metrics")?;
        for received in [&traces, &metrics] {
            assert_eq!(
                header(received, "authorization"),
                Some("Bearer s3cret"),
                "configured headers ride on every request, percent-decoded"
            );
            assert_eq!(header(received, "x-neo-test"), Some("yes"));
            let (content_type, encoding) = match protocol {
                OtlpProtocol::HttpProtobuf => ("application/x-protobuf", Some("gzip")),
                OtlpProtocol::HttpJson => ("application/json", None),
            };
            assert_eq!(header(received, "content-type"), Some(content_type));
            assert_eq!(
                header(received, "content-encoding"),
                encoding,
                "protobuf goes gzipped, JSON plain"
            );
        }
        match protocol {
            OtlpProtocol::HttpProtobuf => check_protobuf(&traces, &metrics)?,
            OtlpProtocol::HttpJson => check_json(&traces, &metrics)?,
        }
        let took = providers.shut_down(Duration::from_secs(1))?;
        assert!(took < Duration::from_secs(1), "{wire:?}: took {took:?}");
        Ok(())
    }

    #[test]
    fn protobuf_over_tcp_arrives_whole() -> Result<(), Box<dyn Error>> {
        both_signals_arrive(Wire::Tcp, OtlpProtocol::HttpProtobuf)
    }

    #[test]
    fn protobuf_over_a_unix_socket_arrives_whole() -> Result<(), Box<dyn Error>> {
        both_signals_arrive(Wire::Unix, OtlpProtocol::HttpProtobuf)
    }

    #[test]
    fn json_over_tcp_arrives_whole() -> Result<(), Box<dyn Error>> {
        both_signals_arrive(Wire::Tcp, OtlpProtocol::HttpJson)
    }

    #[test]
    fn json_over_a_unix_socket_arrives_whole() -> Result<(), Box<dyn Error>> {
        both_signals_arrive(Wire::Unix, OtlpProtocol::HttpJson)
    }

    #[test]
    fn nothing_is_built_without_an_endpoint() -> Result<(), Box<dyn Error>> {
        assert!(
            push(&OtlpConfig::default())?.is_none(),
            "the default pushes nothing: no exporter, no thread, no socket"
        );
        let settings_only = config(&[
            ("OTEL_EXPORTER_OTLP_HEADERS", "authorization=token"),
            ("OTEL_EXPORTER_OTLP_PROTOCOL", "http/json"),
            ("OTEL_METRIC_EXPORT_INTERVAL", "500"),
        ])?;
        assert!(
            push(&settings_only)?.is_none(),
            "settings without an endpoint push nothing"
        );
        Ok(())
    }

    #[test]
    fn each_signal_is_built_only_when_an_endpoint_covers_it() -> Result<(), Box<dyn Error>> {
        let traces = push(&config(&[
            (
                "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
                "https://collector.invalid/v1/traces",
            ),
            (
                "OTEL_EXPORTER_OTLP_HEADERS",
                "authorization=Bearer%20s3cret",
            ),
        ])?)?
        .ok_or("a traces endpoint turns push on")?;
        assert!(traces.spans.is_some() && traces.metrics.is_none());
        let shown = format!("{traces:?}");
        assert!(
            !shown.contains("s3cret"),
            "no Debug in the push door, the exporters' included, shows a header value: {shown}"
        );
        let metrics = push(&config(&[(
            "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
            "http+unix://%2Fnowhere%2Fotel.sock/v1/metrics",
        )])?)?
        .ok_or("a metrics endpoint turns push on")?;
        assert!(metrics.spans.is_none() && metrics.metrics.is_some());
        Ok(())
    }

    #[test]
    fn the_resource_and_the_sampler_follow_the_config() -> Result<(), Box<dyn Error>> {
        let service = Key::new("service.name");
        let defaults = OtlpConfig::default();
        assert_eq!(
            resource(&defaults).get(&service),
            Some(Value::from("browseros-neo"))
        );
        assert_eq!(format!("{:?}", sampler(&defaults)), "ParentBased(AlwaysOn)");
        let config = config(&[
            ("OTEL_SERVICE_NAME", "neo-cell"),
            (
                "OTEL_RESOURCE_ATTRIBUTES",
                "service.name=from-attributes,region=eu",
            ),
            ("OTEL_TRACES_SAMPLER", "parentbased_traceidratio"),
            ("OTEL_TRACES_SAMPLER_ARG", "0.25"),
        ])?;
        let resource = resource(&config);
        assert_eq!(
            resource.get(&service),
            Some(Value::from("from-attributes")),
            "OTEL_RESOURCE_ATTRIBUTES is applied last and wins (wire contract)"
        );
        assert_eq!(resource.get(&Key::new("region")), Some(Value::from("eu")));
        assert_eq!(
            format!("{:?}", sampler(&config)),
            "ParentBased(TraceIdRatioBased(0.25))"
        );
        Ok(())
    }

    #[test]
    fn a_collector_holding_requests_blocks_no_caller_and_the_full_queue_drops()
    -> Result<(), Box<dyn Error>> {
        let (release, held) = watch::channel(false);
        let collector = MockCollector::start(Wire::Tcp, Answer::HoldUntil(held))?;
        let traces = format!("{}/v1/traces", collector.base);
        let providers = Providers::new(&config(&[(
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            traces.as_str(),
        )])?)?;
        let tracer = providers.tracers.tracer("neo-telemetry");
        let start = Instant::now();
        for _ in 0..ENDED {
            tracer.start("neo.op").end();
        }
        let spent = start.elapsed();
        assert!(
            spent < Duration::from_secs(2),
            "ending {ENDED} spans must not wait on a collector that does not answer: {spent:?}"
        );
        // The first batch is at the collector, held; at most the queue's worth waits behind it.
        let mut exported = span_count(&collector.next()?)?;
        release.send_replace(true);
        providers.tracers.force_flush()?;
        while let Ok(received) = collector.received.try_recv() {
            exported += span_count(&received)?;
        }
        assert!(
            (MAX_QUEUED_SPANS..ENDED).contains(&exported),
            "the queue keeps {MAX_QUEUED_SPANS} spans and drops the rest instead of waiting: \
             {exported} of {ENDED} exported"
        );
        providers.shut_down(Duration::from_secs(1))?;
        Ok(())
    }

    /// The review's case: an operator sets only `OTEL_EXPORTER_OTLP_TRACES_HEADERS`, as vendors'
    /// docs say. Spans must carry that credential; metrics keep the general headers and their
    /// own protocol.
    #[test]
    fn each_signal_sends_its_own_headers_and_protocol() -> Result<(), Box<dyn Error>> {
        let collector = MockCollector::start(Wire::Tcp, Answer::Ok)?;
        let providers = Providers::new(&config(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", collector.base.as_str()),
            ("OTEL_EXPORTER_OTLP_HEADERS", "x-general=yes"),
            (
                "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
                "authorization=Bearer%20traces-token",
            ),
            ("OTEL_EXPORTER_OTLP_METRICS_PROTOCOL", "http/json"),
        ])?)?;
        providers.record_a_run();
        providers.tracers.force_flush()?;
        providers.meters.force_flush()?;
        let mut arrived = HashMap::new();
        for _ in 0..2 {
            let received = collector.next()?;
            arrived.insert(received.path.clone(), received);
        }
        let traces = arrived.remove("/v1/traces").ok_or("spans arrive")?;
        let metrics = arrived.remove("/v1/metrics").ok_or("metrics arrive")?;
        assert_eq!(
            header(&traces, "authorization"),
            Some("Bearer traces-token"),
            "spans carry the traces-specific credential"
        );
        assert_eq!(
            header(&traces, "x-general"),
            None,
            "a signal's headers replace the general ones"
        );
        assert_eq!(
            header(&traces, "content-type"),
            Some("application/x-protobuf")
        );
        assert_eq!(header(&metrics, "x-general"), Some("yes"));
        assert_eq!(header(&metrics, "authorization"), None);
        assert_eq!(
            header(&metrics, "content-type"),
            Some("application/json"),
            "metrics take their own protocol"
        );
        providers.shut_down(Duration::from_secs(1))?;
        Ok(())
    }

    /// Exports race callers the whole time (every 1 ms) and every one fails.
    fn a_dead_collector_costs_callers_nothing(wire: Wire) -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let providers = Providers::new(&config(&[
            (
                "OTEL_EXPORTER_OTLP_ENDPOINT",
                nobody_at(wire, &dir)?.as_str(),
            ),
            ("OTEL_BSP_SCHEDULE_DELAY", "1"),
            ("OTEL_METRIC_EXPORT_INTERVAL", "1"),
        ])?)?;
        let tracer = providers.tracers.tracer("neo-telemetry");
        let counter = providers
            .meters
            .meter("neo-telemetry")
            .u64_counter("neo_operations_total")
            .build();
        let start = Instant::now();
        for _ in 0..ENDED {
            tracer.start("neo.op").end();
            counter.add(1, &[KeyValue::new("status", "ok")]);
        }
        let spent = start.elapsed();
        assert!(
            spent < Duration::from_secs(2),
            "{wire:?}: recording must not wait on a dead collector: {spent:?}"
        );
        assert!(
            providers.meters.force_flush().is_ok(),
            "{wire:?}: the transport keeps a failed export from the SDK, which would log it"
        );
        let took = providers.shut_down(Duration::from_secs(1))?;
        assert!(
            took < Duration::from_millis(500),
            "{wire:?}: with nothing listening every final export fails at once and nothing is \
             resent, so shutdown does not wait out its 1 s budget: took {took:?}"
        );
        Ok(())
    }

    #[test]
    fn a_dead_collector_over_tcp_costs_callers_nothing() -> Result<(), Box<dyn Error>> {
        a_dead_collector_costs_callers_nothing(Wire::Tcp)
    }

    #[test]
    fn a_dead_collector_over_a_unix_socket_costs_callers_nothing() -> Result<(), Box<dyn Error>> {
        a_dead_collector_costs_callers_nothing(Wire::Unix)
    }

    /// The final flush reaches a collector that never answers; shutdown still ends on time.
    fn a_hung_collector_leaves_shutdown_in_budget(wire: Wire) -> Result<(), Box<dyn Error>> {
        let collector = MockCollector::start(wire, Answer::Hang)?;
        let providers = Providers::new(&config(&[(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            collector.base.as_str(),
        )])?)?;
        providers.record_a_run();
        let took = providers.shut_down(Duration::from_secs(1))?;
        assert!(
            took < Duration::from_millis(1200),
            "{wire:?}: a collector that never answers must not hold shutdown past its 1 s \
             budget: took {took:?}"
        );
        collector.next()?;
        Ok(())
    }

    #[test]
    fn a_hung_collector_over_tcp_leaves_shutdown_in_budget() -> Result<(), Box<dyn Error>> {
        a_hung_collector_leaves_shutdown_in_budget(Wire::Tcp)
    }

    #[test]
    fn a_hung_collector_over_a_unix_socket_leaves_shutdown_in_budget() -> Result<(), Box<dyn Error>>
    {
        a_hung_collector_leaves_shutdown_in_budget(Wire::Unix)
    }

    #[test]
    fn shutdown_flushes_what_is_pending_when_the_collector_is_up() -> Result<(), Box<dyn Error>> {
        for wire in [Wire::Tcp, Wire::Unix] {
            let collector = MockCollector::start(wire, Answer::Ok)?;
            let providers = Providers::new(&config(&[(
                "OTEL_EXPORTER_OTLP_ENDPOINT",
                collector.base.as_str(),
            )])?)?;
            providers.record_a_run();
            let took = providers.shut_down(Duration::from_secs(1))?;
            assert!(took < Duration::from_secs(1), "{wire:?}: took {took:?}");
            let mut paths = vec![collector.next()?.path, collector.next()?.path];
            paths.sort();
            assert_eq!(
                paths,
                ["/v1/metrics", "/v1/traces"],
                "{wire:?}: the shutdown flush delivers the pending span and metrics"
            );
        }
        Ok(())
    }

    #[test]
    fn a_deadline_armed_mid_export_cuts_the_request_in_flight() -> Result<(), Box<dyn Error>> {
        let collector = MockCollector::start(Wire::Unix, Answer::Hang)?;
        let metrics = format!("{}/v1/metrics", collector.base);
        let providers = Providers::new(&config(&[(
            "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
            metrics.as_str(),
        )])?)?;
        providers.record_a_run();
        let meters = providers.meters.clone();
        let start = Instant::now();
        let flushing = thread::spawn(move || meters.force_flush());
        collector.next()?;
        providers.transport.arm_deadline(Duration::from_millis(300));
        assert!(flushing.join().is_ok(), "the flush thread must not panic");
        let took = start.elapsed();
        assert!(
            took < Duration::from_secs(2),
            "an export in flight when the deadline is armed ends at the deadline, not at its \
             10 s timeout: took {took:?}"
        );
        providers.shut_down(Duration::from_secs(1))?;
        Ok(())
    }

    #[test]
    fn an_unanswered_export_ends_at_its_timeout() -> Result<(), Box<dyn Error>> {
        let collector = MockCollector::start(Wire::Tcp, Answer::Hang)?;
        let metrics = format!("{}/v1/metrics", collector.base);
        let providers = Providers::new(&config(&[
            ("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT", metrics.as_str()),
            ("OTEL_EXPORTER_OTLP_TIMEOUT", "60000"),
            ("OTEL_EXPORTER_OTLP_METRICS_TIMEOUT", "300"),
        ])?)?;
        providers.record_a_run();
        let start = Instant::now();
        assert!(
            providers.meters.force_flush().is_ok(),
            "the transport keeps a failed export from the SDK, which would log it"
        );
        let took = start.elapsed();
        assert!(
            took >= Duration::from_millis(300) && took < Duration::from_secs(2),
            "OTEL_EXPORTER_OTLP_METRICS_TIMEOUT=300 wins over the general 60 s and ends the \
             request at 300 ms: took {took:?}"
        );
        providers.shut_down(Duration::from_secs(1))?;
        Ok(())
    }

    #[test]
    fn a_failed_span_export_is_retried_and_a_metric_push_is_not() -> Result<(), Box<dyn Error>> {
        let failures = Arc::new(AtomicUsize::new(2));
        let collector =
            MockCollector::start(Wire::Tcp, Answer::Unavailable(Arc::clone(&failures)))?;
        let providers = Providers::new(&config(&[(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            collector.base.as_str(),
        )])?)?;
        providers.record_a_run();
        providers.tracers.force_flush()?;
        let attempts: Vec<String> = (0..3)
            .map(|_| collector.next().map(|received| received.path))
            .collect::<Result<_, _>>()?;
        assert_eq!(
            attempts, ["/v1/traces"; 3],
            "a span batch answered 503 is sent up to twice more"
        );
        failures.store(1, Ordering::SeqCst);
        providers.meters.force_flush()?;
        assert_eq!(collector.next()?.path, "/v1/metrics");
        assert!(
            collector
                .received
                .recv_timeout(Duration::from_millis(700))
                .is_err(),
            "a metric push answered 503 is not sent again: the next push carries the same totals"
        );
        providers.shut_down(Duration::from_secs(1))?;
        Ok(())
    }
}
