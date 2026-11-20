//! The pinned OpenTelemetry set builds every piece Neo telemetry needs, on the CI toolchain.
//!
//! This is the version spike, kept as a test so a dependency bump that breaks one of these
//! shapes fails here first: a meter provider read through a shared manual reader (the scrape,
//! rendered by our own code from `ResourceMetrics`' public accessors), a tracer provider, and
//! OTLP/HTTP span and metric exporters for both protocols, each over a client of our own that
//! runs its requests on a thread and runtime it owns (reqwest 0.12, over TCP or a Unix socket),
//! never on a caller's runtime. Nothing here sends a byte: the endpoints are never contacted.

use std::error::Error;
use std::sync::{Arc, Weak, mpsc};
use std::thread;
use std::time::Duration;

use async_trait::async_trait;
use opentelemetry::KeyValue;
use opentelemetry::metrics::MeterProvider;
use opentelemetry::trace::{Tracer, TracerProvider};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_otlp::{
    Compression, MetricExporter, Protocol, SpanExporter, WithExportConfig, WithHttpConfig,
};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData, ResourceMetrics};
use opentelemetry_sdk::metrics::reader::MetricReader;
use opentelemetry_sdk::metrics::{
    InstrumentKind, ManualReader, PeriodicReader, Pipeline, SdkMeterProvider, Temporality,
};
use opentelemetry_sdk::trace::{
    BatchConfigBuilder, BatchSpanProcessor, Sampler, SdkTracerProvider,
};
use tokio::runtime::{Builder, Handle};
use tokio::sync::oneshot;

/// The scrape's reader, shared: the meter provider owns one clone, the renderer collects
/// through another. What `prometheus.rs` will hold.
#[derive(Clone, Debug)]
struct SharedReader(Arc<ManualReader>);

impl MetricReader for SharedReader {
    fn register_pipeline(&self, pipeline: Weak<Pipeline>) {
        self.0.register_pipeline(pipeline);
    }

    fn collect(&self, metrics: &mut ResourceMetrics) -> OTelSdkResult {
        self.0.collect(metrics)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.0.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }

    fn temporality(&self, kind: InstrumentKind) -> Temporality {
        self.0.temporality(kind)
    }
}

/// The shape `transport.rs` fills in: opentelemetry-http's trait over the workspace's reqwest,
/// with the I/O on a current-thread runtime that one owned OS thread drives. The batch processor
/// and periodic reader export from their own threads, outside any runtime; `Handle::spawn` onto
/// the owned runtime and awaiting the `JoinHandle` works from any executor.
#[derive(Debug)]
struct OwnedTransport {
    client: reqwest::Client,
    runtime: Handle,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl OwnedTransport {
    fn start(client: reqwest::Client) -> Result<Self, Box<dyn Error>> {
        let (handle_tx, handle_rx) = mpsc::channel();
        let (stop, stopped) = oneshot::channel::<()>();
        let thread = thread::Builder::new()
            .name("neo-telemetry-spike".to_owned())
            .spawn(move || {
                let runtime = Builder::new_current_thread().enable_all().build();
                match runtime {
                    Ok(runtime) => {
                        // A send error means the test already gave up waiting; nothing to do.
                        let _ = handle_tx.send(Ok(runtime.handle().clone()));
                        // Ends when the transport is dropped: its sender goes with it.
                        let _ = runtime.block_on(stopped);
                    }
                    Err(error) => {
                        let _ = handle_tx.send(Err(error));
                    }
                }
            })?;
        let runtime = handle_rx.recv()??;
        Ok(Self {
            client,
            runtime,
            stop: Some(stop),
            thread: Some(thread),
        })
    }
}

impl Drop for OwnedTransport {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[async_trait]
impl HttpClient for OwnedTransport {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let request = reqwest::Request::try_from(request)?;
        let client = self.client.clone();
        let (status, body) = self
            .runtime
            .spawn(async move {
                let response = client.execute(request).await?;
                let status = response.status();
                Ok::<_, reqwest::Error>((status, response.bytes().await?))
            })
            .await??;
        Ok(Response::builder().status(status).body(body)?)
    }
}

#[test]
fn the_scrape_reads_the_same_meter_provider_through_a_shared_manual_reader()
-> Result<(), Box<dyn Error>> {
    let reader = SharedReader(Arc::new(
        ManualReader::builder()
            .with_temporality(Temporality::Cumulative)
            .build(),
    ));
    let provider = SdkMeterProvider::builder()
        .with_resource(
            Resource::builder_empty()
                .with_service_name("browseros-neo")
                .build(),
        )
        .with_reader(reader.clone())
        .build();
    let meter = provider.meter("neo-telemetry");
    meter
        .u64_counter("neo_spike_total")
        .build()
        .add(2, &[KeyValue::new("status", "ok")]);
    meter
        .f64_histogram("neo_spike_duration_seconds")
        .with_unit("s")
        .with_boundaries(vec![0.1, 1.0])
        .build()
        .record(0.5, &[]);
    let _gauge = meter
        .u64_observable_gauge("neo_spike_open")
        .with_callback(|observer| observer.observe(3, &[KeyValue::new("origin", "agent")]))
        .build();

    let mut collected = ResourceMetrics::default();
    reader.collect(&mut collected)?;
    let mut seen = Vec::new();
    for scope in collected.scope_metrics() {
        for metric in scope.metrics() {
            let summary = match metric.data() {
                AggregatedMetrics::U64(MetricData::Sum(sum)) => sum
                    .data_points()
                    .map(|point| format!("{} {}", metric.name(), point.value()))
                    .collect::<Vec<_>>()
                    .join(","),
                AggregatedMetrics::U64(MetricData::Gauge(gauge)) => gauge
                    .data_points()
                    .map(|point| format!("{} {}", metric.name(), point.value()))
                    .collect::<Vec<_>>()
                    .join(","),
                AggregatedMetrics::F64(MetricData::Histogram(histogram)) => histogram
                    .data_points()
                    .map(|point| {
                        let bounds: Vec<f64> = point.bounds().collect();
                        let counts: Vec<u64> = point.bucket_counts().collect();
                        format!("{} {bounds:?} {counts:?} {}", metric.name(), point.count())
                    })
                    .collect::<Vec<_>>()
                    .join(","),
                _ => format!("{} other", metric.name()),
            };
            seen.push(summary);
        }
    }
    seen.sort();
    assert_eq!(
        seen,
        vec![
            "neo_spike_duration_seconds [0.1, 1.0] [0, 1, 0] 1".to_owned(),
            "neo_spike_open 3".to_owned(),
            "neo_spike_total 2".to_owned(),
        ],
        "a renderer of our own reads names, values, bounds and bucket counts from the reader, \
         and the observable gauge is read at collection time"
    );
    provider.shutdown()?;
    Ok(())
}

#[test]
fn otlp_http_exporters_build_for_both_protocols_over_our_own_transport()
-> Result<(), Box<dyn Error>> {
    let socket_dir = tempfile::tempdir()?;
    let clients = [
        reqwest::Client::builder().build()?,
        reqwest::Client::builder()
            .unix_socket(socket_dir.path().join("otel.sock"))
            .build()?,
    ];
    for client in clients {
        for protocol in [Protocol::HttpBinary, Protocol::HttpJson] {
            let spans = SpanExporter::builder()
                .with_http()
                .with_protocol(protocol)
                .with_endpoint("http://127.0.0.1:9/v1/traces")
                .with_timeout(Duration::from_secs(1))
                .with_compression(Compression::Gzip)
                .with_http_client(OwnedTransport::start(client.clone())?)
                .build()?;
            let metrics = MetricExporter::builder()
                .with_http()
                .with_protocol(protocol)
                .with_endpoint("http://127.0.0.1:9/v1/metrics")
                .with_temporality(Temporality::Cumulative)
                .with_http_client(OwnedTransport::start(client.clone())?)
                .build()?;

            let processor = BatchSpanProcessor::builder(spans)
                .with_batch_config(
                    BatchConfigBuilder::default()
                        .with_max_queue_size(2048)
                        .with_scheduled_delay(Duration::from_millis(500))
                        .build(),
                )
                .build();
            let tracers = SdkTracerProvider::builder()
                .with_sampler(Sampler::ParentBased(Box::new(Sampler::AlwaysOn)))
                .with_span_processor(processor)
                .build();
            let meters = SdkMeterProvider::builder()
                .with_reader(
                    PeriodicReader::builder(metrics)
                        .with_interval(Duration::from_secs(15))
                        .build(),
                )
                .build();
            drop(tracers.tracer("neo-telemetry").start("neo.run"));
            // Shutdown exports what is pending to an endpoint that is never there. Run it on a
            // plain thread and wait with a bound, as the pipeline's shutdown will: the spike
            // only needs it to come back, whatever it reports.
            let (done, finished) = mpsc::channel();
            thread::spawn(move || {
                let traces = tracers.shutdown_with_timeout(Duration::from_secs(1));
                let metrics = meters.shutdown_with_timeout(Duration::from_secs(1));
                let _ = done.send((traces.is_ok(), metrics.is_ok()));
            });
            finished.recv_timeout(Duration::from_secs(10))?;
        }
    }
    Ok(())
}
